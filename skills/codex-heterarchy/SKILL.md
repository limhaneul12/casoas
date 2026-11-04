---
name: codex-heterarchy
description: Gemini 4 Argon, Claude Sonnet/Opus 5.5 Medium, OpenAI Codex 간의 Rust 기반 Tri-Polar 협업, 동적 DAG 실행, 영수증·타임아웃·합의·증거 판독과 hcodex 설치/운영에 사용한다.
---

# Rust Heterarchy Codex

이 스킬은 Rust `hcodex` MCP를 사용해 Argon의 거시 설계, Claude의 정밀 구현, Codex의 적대적 감사를 결합한다. 실제 `receipt`, `evidence_scope`, 역할 상태, 합의 상태만 근거로 판정한다.

## 시작 전 확인

1. `codex_bridge_health`에서 실제 routable 모델과 독립 인증 경계를 확인한다.
2. 대상 workspace와 파일·심볼을 명시한다. 생략 시 Rust relevance/CodeGraph 탐색 결과를 확인한다.
3. write 작업은 DAG의 declared `write_files`, preimage SHA-256, 검증 명령을 고정한다.
4. provider timeout은 같은 ID로 조회한다. 다른 ID로 자동 재시도하지 않는다.

## 16개 MCP 도구

- Codex 실행: `codex_adversarial_audit`, `codex_targeted_synthesize`, `codex_differential_implementations`, `codex_rescue_sandbox`, `codex_git_review`, `codex_session_followup`
- 작업 상태: `codex_job_status`, `codex_job_list`
- 핸드오프: `heterarchy_handoff_create`, `heterarchy_handoff_inbox`, `heterarchy_handoff_resolve`
- 협업: `claude_code_craft`, `heterarchy_triad_orchestrate`, `codex_alexandria_decision_pack`
- DAG: `heterarchy_dag_ulw`
- 상태: `codex_bridge_health`

## 절대 판정 규칙

- `requested_model != actual_model`이면 성공으로 취급하지 않는다.
- `rejected_pre_dispatch`는 provider 호출이 발생하지 않은 확정 실패다.
- `timed_out_unknown`은 성공도 실패도 아니다. `codex_job_status(job_id=...)`로 조회·취소한다.
- 핸드오프 생성은 `queued`이며 실행 완료나 합의가 아니다.
- 필수 Argon·Claude·Codex 역할 모두가 `succeeded`, 비어 있지 않은 검토 artifact, 명시적 `PASS|WARN|BLOCK`를 가져야 합의를 계산할 수 있다.
- `BLOCK`은 red-team block이며 승인으로 바꾸지 않는다. 모든 역할이 `WARN`이어도 `review_approved=false`다.
- caller가 전달한 테스트 명령은 `caller_reported`이고 `test_execution=not_run`이다. bridge가 직접 실행한 성공 영수증만 verified다.

## 권장 라우팅

- 아키텍처·최종 심의: `gemini-4-argon-high`
- 구현: `claude-sonnet-5-5-medium` 또는 `claude-opus-5-5-medium`
- 감사·합성: `gpt-6-terra|luna|sol|astra`, `gpt-6.1-terra|sol`
- Claude low/high tier, GPT 5.x, provider/engine mismatch는 실행 전에 거부한다.

## Triad

`heterarchy_triad_orchestrate`의 `pipeline`은 Claude → Codex → Argon 순서다. `council`은 역할을 병렬 실행하고 판정 충돌을 별도 상태로 남긴다.

- `max_rounds=1..3`: WARN/BLOCK 수리·재감사를 bounded하게 수행한다.
- `resume_collaboration_id`: workspace, task digest, 파일 digest, snapshot fence가 일치하는 성공 역할만 재사용한다.
- `run_verification_gates=true`: repository canonical gate를 bridge가 직접 실행한다.
- Argon handoff가 pending이거나 어떤 역할이 미확정이면 `partial` + `incomplete_no_consensus`다.

## DAG-ULW++

`heterarchy_dag_ulw`는 목표와 실제 source structure로 4~8 nodes, 2~4 waves를 만든다.

- plan-only로 topology와 WAW/RAW/WAR/cycle/path hazard를 먼저 확인한다.
- wave/layer는 표시용이다. 검증된 의존성이 충족된 노드를 빈 슬롯에 즉시 배치하며, 공유 workspace의 쓰기는 독점 실행한다.
- 후속 노드는 선행 노드의 검증 영수증·제한된 결과 요약을 받는다. resume도 같은 영수증을 사용한다.
- TUI는 실행·STOPPED·검증·경과 시간을 실시간 표시한다. provider 토큰·도구 호출 본문 스트리밍은 아직 제공하지 않는다.
- interactive 실행은 검증된 spawn plan을 만든 뒤 같은 canonical request path를 사용한다.
- write lane의 Claude artifact는 `{edits:[{path,preimage_sha256,new_content}]}` closed JSON이어야 한다.
- traversal, symlink, scope 밖 파일, stale preimage, provider의 선행 mutation을 거부한다.
- provider 완료는 `STOPPED`일 뿐이다. 실제 gate와 전체 manifest 검증 뒤에만 `SETTLED`다.
- checkpoint resume은 plan/config/source identity가 일치할 때만 settled node를 재사용한다.

## 설치와 CLI

```bash
cargo install --locked --path crates/hcodex
hcodex setup
hcodex status
hcodex tui
hcodex restart
```

수동 MCP 설정은 `command="hcodex"`, `args=["mcp"]`를 사용한다. Python module이나 `.venv`를 MCP 진입점으로 등록하지 않는다.

CLI 주요 명령: `mcp`, `tui [-c]`, `status`, `jobs`, `inbox`, `ask`, `sonnet`, `opus`, `argon`, `flash`, `dag-ulw`, `dag-run`, `dag-mode`, `dag-exit`, `models`, `skills`, `on`, `off`, `restart`, `setup`, `sync`, `prune`.

## Rust 개발 규칙

저장소 작업 시 `AGENTS.md`, `.agents/rust_dev_harness/PROJECT_PROFILE.md`, `HARNESS.toml`, `HARNESS_MANIFEST.toml`, `rules/README.md`를 먼저 읽는다.

- Rust 2024, repository toolchain, safe-only를 지킨다.
- 외부 입력은 closed typed serde 경계로 검증한다.
- task/process는 supervisor가 소유하고 취소·타임아웃 시 kill/reap한다.
- SQL은 `crates/hcodex/sql/`의 prepared statement만 사용한다.
- 실행하지 않은 검증은 PASS로 보고하지 않는다.

Canonical 검증:

```bash
make ci
```

이 프로젝트의 Python 구현·테스트·가상환경·백업·검증 스크립트는 완전히 제거됐다. Rust 제품·CI는 Python에 의존하지 않는다. 과거 데이터 호환용 SQLite fixture와 JSON/로그 영수증은 보존하며, 외부 Python workspace를 분석·검증하는 기능은 별개로 지원한다.
