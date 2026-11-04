# 조직 작업판과 Review Council

`hcodex`는 작업 상태, 노드 인계, 검토 투표를 SQLite 작업 기록 위에 투영합니다. 새 메시지 서버나 투표 DB를 만들지 않습니다.

## 운영 흐름

```text
board (저장 상태 조회)
  → council (실제 세 역할 검토)
  → votes (영수증·판정·소스 최신성 조회)
  → handoff (다음 DAG 노드가 검증된 이력 수신)
```

### 작업판 조회

```bash
hcodex --workspace /path/to/repo board --limit 20
```

TUI에서는 `/board [limit]`를 사용합니다. 작업판은 현재 workspace의 campaign, checkpoint, node handoff, Review Council 상태를 제한된 형태로 보여줍니다. raw provider 출력이나 private reasoning을 반환하지 않으며 모델을 호출하지 않습니다.

### Review Council 실행

```bash
hcodex --workspace /path/to/repo council --file src/lib.rs --file Cargo.toml "이 변경의 안정성·범위·검증 근거를 검토하라"
```

TUI에서는 `/council --file src/lib.rs --file Cargo.toml -- <proposal>`을 사용합니다. `--` 앞에는 검토할 workspace 상대 경로만 둡니다. TUI 명령의 파일 경로에는 공백을 사용할 수 없으며, native CLI에서는 `--file "path with spaces.rs"`처럼 인용할 수 있습니다.

파일 범위 없이 `hcodex council "정책 제안"` 또는 `/council <proposal>`을 실행할 수도 있습니다. 이는 정책·아키텍처 토론에는 유효하지만 source freshness가 `unknown`이므로 현재 코드 승인 근거가 아닙니다.

이 명령은 기존 `heterarchy_triad_orchestrate`의 `council` 모드로 Argon High, Claude Sonnet Medium, Codex 6.1 Sol을 실행합니다. 검토 게이트를 실행했다고 주장하지 않으며, queued/running 상태는 투표나 승인으로 세지 않습니다.

현재 Argon/Claude RPC 요청은 텍스트 프롬프트를 받으며 별도 child-agent 도구 영수증을 제공하지 않습니다. DAG 내부 병렬 실행도 현재 최대 6개이므로 “8개 에이전트”를 제품 보장으로 해석하지 않습니다.

### 투표 판독

```bash
hcodex --workspace /path/to/repo votes cdx-collaboration-id
```

TUI에서는 `/votes <collaboration_id>`를 사용합니다. v1 투표는 A/B 선택이 아니라 **검토 판정**입니다.

| 판정 | 의미 |
|---|---|
| `PASS` | 해당 역할이 검토 범위 안에서 차단 사유를 찾지 않음 |
| `WARN` | 조건·미검증 범위·잔여 위험이 있음 |
| `BLOCK` | red-team 차단 사유. 다수결로 덮지 않음 |

모든 필수 역할이 실제 성공 영수증, requested/actual model 일치, 비어 있지 않은 구조화 검토 artifact를 가져야 quorum을 계산합니다. 역할이 누락되거나 unknown이면 `incomplete_no_consensus`이며, 전원이 `WARN`이어도 `review_approved=false`입니다. 같은 실행 ID를 여러 독립 표로 세지 않습니다.

`votes`는 저장 당시 판정과 현재 소스 상태를 분리합니다.

- `current_scoped`: 검토가 명시한 제한된 파일의 현재 SHA-256이 저장값과 일치합니다.
- `stale`: 그 제한된 파일 중 하나 이상이 변경되거나 사라졌습니다.
- `unknown`: 파일 범위가 없거나 불완전하고, 경로·해시·파일 수·바이트 한도를 검증할 수 없습니다.

이 상태는 `files_inspected_digests` 범위만 다룹니다. `current_scoped`도 전체 workspace, provider E2E, native UI 또는 배포 readiness의 증거는 아닙니다. `stale`과 `unknown` 표는 역사 기록으로 조회할 수 있지만 현재 소스 승인에는 사용하지 않습니다.

## 노드 인계 계약

`SETTLED` 노드는 기존 checkpoint 안에 하나의 `NodeHandoff`를 저장합니다.

- system facts: actual model, changed paths, pre/post manifest digest, gate proof
- agent report: 제한된 요약, 결정 사항, caveat
- causal refs: 직접 부모와 필요한 조상 checkpoint의 campaign/node/content digest

system facts는 검증 증거이고 agent report는 에이전트 주장입니다. hcodex는 둘을 별도로 표시하고 raw stdout/stderr, prompt, private reasoning 전체를 저장하지 않습니다.

같은 checkpoint ID와 동일 내용은 정확 재생으로 처리해 attempt, timestamp, metadata를 바꾸지 않습니다. 다른 내용이나 범위는 conflict입니다. 저장 완료를 확인하기 전에는 후속 노드를 실행하지 않습니다. 부모·조상 기록의 범위나 해시가 맞지 않거나 필수 컨텍스트가 누락·초과되면 `context_not_ready`로 중단합니다.

## 요청과 미확정 결과

새 provider 요청은 workspace, 도구, 모델, engine과 의미 있는 입력을 묶은 불변 identity를 저장합니다. 현재 파일이 나중에 바뀌어도 허가된 과거 결과 readback은 유지하지만, 같은 ID를 다른 요청에 재사용하면 거절합니다.

`timed_out_unknown`은 성공이나 실패가 아닙니다. 완료·실패·취소·미확정 record는 일반 retry 플래그로 다시 dispatch하지 않으며 원래 ID로 조회합니다. fingerprint가 없는 역사 record는 읽을 수 있지만 새 identity를 소급해서 만들지 않습니다.

## 조회 전용 복구 진단

```bash
hcodex --workspace /path/to/repo --state-root /path/to/existing-state recovery cdx-campaign-id
```

TUI에서는 `/recovery <job_id>`를 사용합니다. 작업판에서 확인한 정확한 작업 ID를 지정하면 기존 작업·export 기록, `lifecycle.json`, 현재 Git worktree 등록과 경로를 제한된 범위에서 대조합니다. 다른 workspace의 작업은 조회 대상으로 인정하지 않습니다.

- `present_registered`: 현재 디렉터리와 Git 등록이 일치하는 관측입니다.
- `observed_removed`: 과거 제거 기록과 현재 경로·Git 등록 부재가 일치하는 관측입니다.
- `unknown`: 자료가 없거나 손상·변동·범위 초과·모순이 있어 현재 상태를 확인할 수 없습니다.

과거 `verified`·`removed`·프로세스 종료 영수증은 현재 관측과 별도로 표시합니다. Compact lifecycle만으로 재시작 후 소유권을 증명하지 않으며, 이 화면은 삭제 안전성이나 재개 권한을 만들지 않습니다. 실제 재개는 별도의 영속 ownership 증거와 frontier를 검증합니다. 여러 자원을 하나의 원자적 snapshot으로 읽었다고 보장하지 않습니다.

진단은 별도 read-only SQLite 연결을 사용하며 `Store::open`의 초기화·오래된 Running 정산을 호출하지 않습니다. 없는 state/DB를 만들지 않고, 행·스키마를 수정하거나 checkpoint를 요청하지 않습니다. SQLite 엔진의 정상적인 잠금용 WAL/SHM 생성·조정·정리는 가능하므로 파일 전체의 바이트 불변을 뜻하지 않습니다. live WAL을 무시할 수 있는 `immutable=1`이나 writable 재시도로 우회하지 않습니다.

이 조회 명령은 새 DB, transcript 수집, 자동 삭제·재전송·worktree resume 권한을 추가하지 않습니다. raw provider 출력·prompt·자유 형식 오류 대신 필요한 ID·상태·모델·검증 결과만 표시합니다. `--worktree-name`과 함께 실행하면 조회 중 새 작업 공간이 만들어지지 않도록 거절합니다.

`hcodex resume <campaign_id>`와 TUI `/resume <campaign_id>`는 [worktree 재개 계약](dag-worktree-mode.md)을 따르는 별도 실행 경로입니다. `status`/`board`의 `runtime_identity`는 소스·설치 파일·현재 프로세스·등록된 MCP의 증거 범위를 분리하며, 과거 필드나 source→binary/CI 연결 증거가 없으면 미검증으로 표시합니다.

## 설치본 확인

소스와 실행 중 MCP는 서로 다른 버전일 수 있습니다. Review Council health만으로 CLI 기능 설치를 판단하지 말고 실제 선택된 바이너리를 확인합니다.

```bash
hcodex --help
hcodex board --help
hcodex council --help
hcodex votes --help
hcodex recovery --help
```

`--worktree-name`도 같은 방식으로 확인합니다. 스킬 파일 갱신은 바이너리 설치나 MCP 재시작의 증거가 아닙니다.

## 저장 안정성

프로젝트는 기존 bundled SQLite 의존성을 `rusqlite 0.39`로 갱신해 SQLite 3.51.3을 사용합니다. 이전 SQLite 3.51.1에서 연결 open/close 경쟁으로 관측·재현된 교착을 해결하는 upstream 수정이 포함됩니다. 이는 DB actor나 새 저장소 도입이 아닙니다. 전체 제품 안정성은 현재 소스에 대한 canonical `make ci`와 실제 CLI/TUI 검증을 별도로 통과해야 합니다.

## 2026-11-04 검증 기록

- 현재 소스의 `make ci`: 318개 통과, 실패·무시 0개. 규칙·manifest·format·check·strict Clippy 포함.
- 실제 CLI file-scoped Council: Argon High, Sonnet Medium, Codex 6.1 Sol의 모델 영수증과 세 PASS 표를 조회했다. 한 파일의 `current_scoped` 판정이며 전체 저장소 승인 증거가 아니다.
- 그 파일 변경 후 같은 투표는 `stale`이 되고 `current_review_approved=false`였다. 원래 투표 이력은 보존됐다.
- 실제 PTY에서 `/board`와 `/votes`의 작업 행, 실제 모델·표, 소스 최신성 표시를 확인했다.
- 별도 clean Git 저장소에서 실제 Sonnet 두 노드와 Codex 두 노드의 worktree DAG를 실행했다. 코드 수정·신규 파일 생성·실제 Cargo 검증을 거쳐 네 노드가 `SETTLED`가 됐다.
- 1번의 인계 표식은 파일과 2번 요약에 넣지 않았으며 3번의 응답에서 확인됐다. 호스트가 조상 기록을 읽어 전달하는 경로를 검증했다.
- 체크포인트와 전체 결과 export 뒤 네 노드 worktree가 정리됐고 최종 소스 결과는 남았다.
- 첫 실제 DAG에서는 아직 없는 신규 write 경로를 기존 read 소스로 처리하는 오류를 발견했다. 실패 기록·worktree를 보존하고, 엄격한 read 검증을 유지한 채 신규 write만 typed `ABSENT` 계약으로 전달하도록 수정·회귀 검증한 후 별도 실행을 완료했다.

로컬 CLI를 최적화 빌드로 갱신했다. 이미 실행 중이던 MCP 클라이언트 프로세스는 이 검증에서 강제 재시작하지 않았다. 외부 비동기 코드 감사 한 건의 `timed_out_unknown`은 원래 ID로 보존했으며 승인으로 계산하지 않았다. 소스 독립 재검토와 위 실행 증거를 구분한다.
