---
name: codex-heterarchy
description: Rust hcodex로 Argon·Claude Medium·Codex의 Tri-Polar 협업, 동적 DAG·노드 인계·조직 작업판·Review Council을 운영하고 실제 모델 영수증·소스 최신성·미확정 결과를 판독한다.
---

# Rust Heterarchy Codex

이 스킬은 Rust `hcodex` MCP를 사용해 Argon의 거시 설계, Claude의 정밀 구현, Codex의 적대적 감사를 결합한다. 실제 `receipt`, `evidence_scope`, 역할 상태, 합의 상태만 근거로 판정한다.

## 시작 전 확인

1. `codex_bridge_health`에서 실제 routable 모델과 독립 인증 경계를 확인한다.
2. 대상 workspace와 파일·심볼을 명시한다. 생략 시 Rust relevance/CodeGraph 탐색 결과를 확인한다. CLI 기능은 실제 선택된 `hcodex --help`와 대상 소스로 확인한다. 모델 연결 health만으로 새 CLI 기능까지 설치됐다고 판단하지 않는다.
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
- direct/serial checkpoint resume은 plan/config/source identity가 일치할 때만 settled node를 재사용한다. worktree resume는 영속 소유권과 검증된 frontier가 있는 기존 캠페인에 한정한다.

## 이름 있는 세션과 노드 worktree

설치본의 `hcodex --help`에 `--worktree-name`이 있는지 먼저 확인한다. 소스에 기능이 있어도 실행 파일·기존 MCP 프로세스가 갱신됐다는 뜻은 아니다. 스킬 갱신과 설치/재시작 결과는 따로 검증한다.

```bash
hcodex --worktree-name feature-auth
```

이 명령은 이름 있는 작업 공간을 생성/다시 열고 TUI에 진입한다. 그 세션에서 `/dag-mode`를 켜거나 DAG를 실행하면 노드별 worktree 모드가 기본 적용된다. 계획만 보려면 `hcodex --worktree-name feature-auth dag-ulw --plan-only "목표"`를 사용한다. 이름은 영문·숫자·`-`·`_` 1~64자이며 원본 저장소와 외부 state root별로 구분한다. 이름은 Git branch 자동 생성 요청이 아니다.

이름 있는 세션을 사용하지 않을 때는 `/dag-run --worktree <goal>` 또는 `hcodex dag-run --worktree "목표"`로 명시한다. MCP의 `heterarchy_dag_ulw`에는 다음 spawn plan과 `plan_only=false`를 전달한다.

```json
{"plan_summary":"검증된 노드별 작업 공간","hazard_acknowledgment":true,"isolation_mode":"worktree","max_parallel_per_wave":3}
```

- 생성 전 **원본 작업 폴더 자체**의 staged·unstaged·untracked 상태가 모두 clean해야 한다. 기준 커밋만 clean한 것으로 대체하거나 dirty 파일 복사·stash·reset·자동 commit으로 우회하지 않는다. 이름 있는 공간을 다시 열 때 원본은 clean이어야 하며, 그 공간의 변경은 그대로 보존한다. 새 DAG 시작 시에는 해당 공간도 clean해야 한다.
- hcodex가 모든 pending 노드의 worktree를 clean한 시작 시점에 미리 생성하고 에이전트에 배정한다. 에이전트에게 임의 Git 분기·병합·삭제를 맡기지 않는다.
- `1 → 2 → 3`에서는 1의 검증된 수정·새 파일을 포함한 전체 소스 상태가 2에, 2의 변경까지 누적된 상태가 3에 전달된다. 파일 baseline과 검증 영수증이 함께 필요하며 요약문만으로 상태 전달을 대신하지 않는다. 실패·미확정 선행 결과는 실행 의존성을 충족하지 않는다.
- 노드 worktree와 빌드 산출물은 분리한다. canonical 반영은 직렬화하며, isolated gate뿐 아니라 반영 대상의 gate와 manifest도 통과해야 SETTLED다. 이름 있는 세션에서는 이 반영 대상이 이름 있는 작업 공간이며 원래 parent checkout은 자동 수정하지 않는다.
- 체크포인트·전체 결과 export의 저장, 소유 process group 종료, 정확한 작업 공간/등록 identity와 파일 증거가 확인된 노드만 정리한다. `owned_processes_closed`는 해당 소유 그룹의 부재 증거이며 임의로 그룹을 탈출한 자손까지 보장하는 OS sandbox 증거가 아니다.
- 실패·unknown·예상 밖 변경·저장 오류는 원래 ID와 경로를 보존한다. `cleanup_unknown`은 삭제/보존을 단정하지 말고 DAG 결과와 `worktree_state_path`·lifecycle 보고서로 확인한다. TTL이나 오류 후 강제 삭제로 종료를 꾸미지 않는다.
- 최종 변경이 모인 이름 있는 세션은 자동 삭제하지 않는다. 다음 실행을 위해 clean 상태가 필요해도 사용자의 commit/정리 권한 범위를 유지한다. 같은 이름으로 재열기는 기존 소스를 초기화하거나 parent HEAD로 갱신하지 않는다.
- 첫 버전은 Unix와 일반 Git checkout을 대상으로 한다. submodule·sparse·소스 symlink, 원본 내부 state root 등 지원하지 않는 입력은 오류이며 direct로 자동 전환하지 않는다. worktree는 네트워크·인증·공유 Git 저장소의 격리가 아니다.
- worktree `resume_dag_id`로 미확정 provider를 새 ID에 재전송하지 않는다. 원래 캠페인/provider ID를 조회한다. 동일 worktree 요청 ID는 기존 결과 조회용이며 workspace/config가 달라지면 거절한다.

`hcodex --workspace /path/to/existing/workspace resume <campaign_id>` 또는 TUI `/resume <campaign_id>`는 저장된 정확한 계획·모델·스킬 본문·요청 설정으로 새 continuation을 만든다. `/dag-run --resume <campaign_id>`도 같은 경로이며 새 목표를 덧붙이지 않는다. Named session은 먼저 정상적으로 열어 그 안에서 `/resume`하거나 기존 세션 경로를 `--workspace`로 지정한다. 초기 resume와 `--worktree-name` 조합은 새 공간을 만들기 전에 거절한다.

- 새 worktree를 만들지 않고 원래 미리 생성한 retained 노드를 재연다. HEAD/index와 고정된 최신 소스·private 인벤토리, Git/파일 identity, owner digest 및 frontier를 검증한다. 검증된 자기 변경도 저장값과 전수 일치해야 한다.
- 준비·admission은 효과 전에 기록한다. 체크포인트 부재만으로 미실행을 추정하지 않으며, 원래 owner에 속한 stable provider ID가 이미 있으면 pending 노드를 다시 보내지 않는다.
- checkpointed `SETTLED`와 확실히 미착수인 pending만 재개한다. 부분 반영·Preparing/Admitted·유실/모순·부분 cleanup은 보존 후 거절한다. 과거 owner 없는 worktree 기록에 권한을 소급하지 않는다.
- 원본이 clean해야 하는 새 생성 조건은 유지된다. `ownership.json`의 bounded 인벤토리와 기존 계획 checkpoint의 한 개 request recipe는 재개 증거이며 transcript 저장소가 아니다.

소스가 있는 경우 자세한 계약은 `docs/dag-worktree-mode.md`를 확인한다. 구간 패치와 `.hcodex/changes/` 단일 파일 복구는 별도 라이브러리 실험이다. 실제 DAG의 쓰기 artifact는 위 full-file `edits` 계약이며, 스킬이 자동 파일 적용·복구나 배포 권한을 추가하지 않는다.

## 조직 작업판과 Review Council

작업 진행과 검토를 채팅 transcript에만 의존하지 않는다. CLI 또는 TUI에서 저장된 상태를 조회하고 실제 Review Council 판정을 분리한다.

```bash
hcodex --workspace /path/to/repo board --limit 20
hcodex --workspace /path/to/repo council --file src/lib.rs --file README.md "검토 제안"
hcodex --workspace /path/to/repo votes cdx-collaboration-id
hcodex --workspace /path/to/repo recovery cdx-campaign-id
```

TUI 명령은 `/board [limit]`, `/council --file relative.rs -- <proposal>`, `/votes <collaboration_id>`다. TUI의 file path에는 공백을 쓸 수 없고 native CLI에서는 인용할 수 있다. 파일 범위 없는 `council <proposal>`은 정책 토론에는 사용할 수 있지만 source freshness가 `unknown`이며 현재 소스 승인 근거가 아니다.

- `board`는 현재 workspace의 campaign, checkpoint, node handoff, council 상태를 읽는다. 모델 호출이나 새 plan 생성을 하지 않는다.
- `council`은 기존 `heterarchy_triad_orchestrate(mode=council)`를 사용한다. queued/running 역할은 투표가 아니다.
- `votes`는 역할별 실제 job ID, requested/actual model, 구조화 verdict, artifact identity를 확인한다. 같은 실제 실행을 여러 독립 표로 세지 않는다.
- v1은 `PASS|WARN|BLOCK` **검토 판정**이다. A/B 선택 투표가 아니다. 필수 역할의 quorum이 필요하고 `BLOCK`은 다수결로 덮지 않으며 전원 `WARN`도 승인되지 않는다.
- `current_scoped|stale|unknown`은 검토가 실제로 열거한 파일 범위의 현재 SHA-256만 판독한다. `current_scoped`도 전체 workspace·provider·GUI·배포 검증이 아니다. stale/unknown 표는 현재 승인 근거가 아니다.

`SETTLED` 노드는 기존 checkpoint 안에 한 개의 hash-bound 인계 기록을 남긴다. verified system facts와 agent-reported decisions/caveats를 구분하고, 직접 부모·필요한 조상은 scoped checkpoint reference로 연결한다. hcodex가 참조를 검증해 모든 provider 프롬프트를 구성한다. 필수 기록이 없거나 입력 예산을 넘으면 `context_not_ready`이며 이력을 조용히 자르지 않는다.

동일 checkpoint의 정확한 재생은 기존 record를 바꾸지 않고, 같은 ID의 다른 내용·범위는 conflict다. 완료·실패·취소·미확정 provider job은 일반 retry 플래그로 다시 실행하지 않고 원래 ID로 조회한다. history record는 읽되 없는 request identity를 소급 생성하지 않는다.

현재 Argon/Claude RPC는 `{model,prompt}` 텍스트 호출이며 child-agent 도구 영수증을 노출하지 않는다. 모델이 위임했다고 서술해도 독립 실행 증거로 세지 않는다. DAG 병렬 실행도 현재 최대 6개이므로 8-agent 조직을 제품 보장으로 보고하지 않는다.

상세 운영 계약은 소스 저장소의 `docs/organization-workbench.md`를 확인한다.

## 복구 상태 조회

`hcodex recovery <job_id>` 또는 TUI `/recovery <job_id>`는 정확한 기존 작업 ID의 상태와 worktree 관측을 조회한다. `Store::open` 초기화·stale-running 정산, 모델 호출, 삭제·재전송·resume를 수행하지 않는다. 없는 state/DB는 만들지 않으며 `--worktree-name`과의 조합은 거절한다.

`present_registered`·`observed_removed`·`unknown`은 현재 관측이며 과거 verified/removed·프로세스 종료 영수증과 구별한다. 관측은 소유권·정리 안전성·재개 승인이 아니다. 행·스키마를 수정하거나 checkpoint를 요청하지 않지만 SQLite의 정상적인 WAL/SHM 잠금 조정은 가능하다. 여러 자원의 원자적 snapshot이나 모든 파일의 바이트 불변을 주장하지 않는다. 자료 누락·변동·모순은 unknown으로 남기고 원래 ID를 유지한다.

## 설치와 CLI

`status`/`board`의 `runtime_identity`는 소스·설치 파일·현재 프로세스·등록된 MCP 시작 관측을 구별한다. 디스크 파일 교체 후 해시는 실행 중 이미지 증거가 아니고 package version 일치도 코드 일치 증거가 아니다. 과거 필드 부재와 source→binary/CI 영수증 부재는 미검증으로 남긴다. 이 조회로 MCP를 재시작하지 않는다. 새 health 구현의 선택적 `workspace_dir`는 소스 관측 범위를 지정하며 기존 인자 없는 호출도 유지된다.

```bash
cargo install --locked --path crates/hcodex
hcodex setup
hcodex status
hcodex tui
hcodex restart
```

수동 MCP 설정은 `command="hcodex"`, `args=["mcp"]`를 사용한다. Python module이나 `.venv`를 MCP 진입점으로 등록하지 않는다.

CLI 주요 명령: `mcp`, `tui [-c]`, `status`, `jobs`, `inbox`, `board`, `council`, `votes`, `recovery`, `resume`, `ask`, `sonnet`, `opus`, `argon`, `flash`, `dag-ulw`, `dag-run`, `dag-mode`, `dag-exit`, `models`, `skills`, `on`, `off`, `restart`, `setup`, `sync`, `prune`.

## Rust 개발 규칙

저장소 작업 시 `AGENTS.md`, `.agents/README.md`, `.agents/rust_dev_harness/` 아래의 `PROJECT_PROFILE.md`, `HARNESS.toml`, `HARNESS_MANIFEST.toml`, `rules/README.md`를 먼저 읽는다.

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
