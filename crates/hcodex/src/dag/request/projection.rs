use serde::Serialize;
use tokio::sync::mpsc;

use super::DagExecutionEvent;

/// Live updates are disposable projections; checkpoints remain authoritative.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub(super) struct ProjectionDelivery {
    pub(super) observer_attached: bool,
    pub(super) dropped_intermediate_events: usize,
    pub(super) undelivered_settlements: usize,
    pub(super) observer_closed: bool,
    pub(super) resync_required: bool,
}

pub(super) struct LiveProjection {
    observer: Option<mpsc::Sender<DagExecutionEvent>>,
    pending_drops: usize,
    delivery: ProjectionDelivery,
}

impl LiveProjection {
    pub(super) fn new(observer: Option<mpsc::Sender<DagExecutionEvent>>) -> Self {
        let delivery = ProjectionDelivery {
            observer_attached: observer.is_some(),
            ..ProjectionDelivery::default()
        };
        Self {
            observer,
            pending_drops: 0,
            delivery,
        }
    }

    /// Never let a progress update block the checkpoint writer. Once lag is
    /// observed, suppress further intermediates until the gap can be announced.
    pub(super) fn intermediate(&mut self, event: DagExecutionEvent) {
        let Some(observer) = &self.observer else {
            return;
        };
        if self.pending_drops > 0 || self.delivery.observer_closed {
            self.dropped();
            return;
        }
        match observer.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.dropped(),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.delivery.observer_closed = true;
                self.dropped();
            }
        }
    }

    /// The caller has already persisted the checkpoint and commit notification.
    pub(super) async fn settled(&mut self, event: DagExecutionEvent) {
        self.flush_lag().await;
        if let Some(observer) = &self.observer
            && (self.delivery.observer_closed || observer.send(event).await.is_err())
        {
            self.delivery.observer_closed = true;
            self.delivery.resync_required = true;
            self.delivery.undelivered_settlements =
                self.delivery.undelivered_settlements.saturating_add(1);
        }
    }

    /// Finalizing a projection never waits on a full observer. The delivery
    /// summary retains the gap even when the tail marker cannot be sent. That
    /// summary is included only when the caller produces an execution report;
    /// interrupted executions may have checkpoints without a final report.
    pub(super) fn finish(mut self) -> ProjectionDelivery {
        if self.pending_drops > 0
            && !self.delivery.observer_closed
            && let Some(observer) = &self.observer
        {
            match observer.try_send(DagExecutionEvent::ProjectionLagged {
                dropped_events: self.pending_drops,
            }) {
                Ok(()) => self.pending_drops = 0,
                Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.delivery.observer_closed = true;
                }
            }
        }
        self.delivery
    }

    fn dropped(&mut self) {
        self.pending_drops = self.pending_drops.saturating_add(1);
        self.delivery.dropped_intermediate_events =
            self.delivery.dropped_intermediate_events.saturating_add(1);
        self.delivery.resync_required = true;
    }

    async fn flush_lag(&mut self) {
        if self.pending_drops == 0 || self.delivery.observer_closed {
            return;
        }
        if let Some(observer) = &self.observer {
            let event = DagExecutionEvent::ProjectionLagged {
                dropped_events: self.pending_drops,
            };
            if observer.send(event).await.is_ok() {
                self.pending_drops = 0;
            } else {
                self.delivery.observer_closed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn healthy_projection_has_no_lag_marker_and_disabled_projection_is_distinct()
    -> crate::Result<()> {
        let (sender, mut receiver) = mpsc::channel(2);
        let mut projection = LiveProjection::new(Some(sender));
        projection.intermediate(DagExecutionEvent::WaveStarted {
            wave_index: 1,
            node_ids: vec!["a".to_owned()],
        });
        projection.intermediate(DagExecutionEvent::WaveFinished { wave_index: 1 });
        let delivery = projection.finish();
        assert_eq!(
            delivery,
            ProjectionDelivery {
                observer_attached: true,
                ..ProjectionDelivery::default()
            }
        );
        assert!(matches!(
            receiver.recv().await,
            Some(DagExecutionEvent::WaveStarted { wave_index: 1, .. })
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(DagExecutionEvent::WaveFinished { wave_index: 1 })
        ));
        assert!(receiver.recv().await.is_none());
        assert_eq!(
            LiveProjection::new(None).finish(),
            ProjectionDelivery::default()
        );
        Ok(())
    }

    #[tokio::test]
    async fn trailing_gap_counts_suppressed_updates_without_a_later_settlement() -> crate::Result<()>
    {
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .send(DagExecutionEvent::WaveFinished { wave_index: 0 })
            .await?;
        let mut projection = LiveProjection::new(Some(sender));
        projection.intermediate(DagExecutionEvent::WaveFinished { wave_index: 1 });
        // Even if capacity returns, do not pass an intermediate before its gap.
        assert!(matches!(
            receiver.recv().await,
            Some(DagExecutionEvent::WaveFinished { wave_index: 0 })
        ));
        projection.intermediate(DagExecutionEvent::WaveFinished { wave_index: 2 });
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let delivery = projection.finish();
        assert!(matches!(
            receiver.recv().await,
            Some(DagExecutionEvent::ProjectionLagged { dropped_events: 2 })
        ));
        assert!(receiver.recv().await.is_none());
        assert_eq!(delivery.dropped_intermediate_events, 2);
        assert!(delivery.resync_required);
        assert!(!delivery.observer_closed);
        Ok(())
    }

    #[tokio::test]
    async fn full_tail_observer_cannot_hold_final_result_open() -> crate::Result<()> {
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .send(DagExecutionEvent::WaveFinished { wave_index: 0 })
            .await?;
        let mut projection = LiveProjection::new(Some(sender));
        projection.intermediate(DagExecutionEvent::WaveFinished { wave_index: 1 });
        let delivery = projection.finish();
        assert_eq!(delivery.dropped_intermediate_events, 1);
        assert!(delivery.resync_required);
        assert!(!delivery.observer_closed);
        assert!(matches!(
            receiver.recv().await,
            Some(DagExecutionEvent::WaveFinished { wave_index: 0 })
        ));
        assert!(receiver.recv().await.is_none());
        Ok(())
    }
}
