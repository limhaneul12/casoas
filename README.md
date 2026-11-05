# hcodex — Rust Heterarchy Codex

[![Rust CI](https://github.com/limhaneul12/casoas/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/limhaneul12/casoas/actions/workflows/ci.yml)
![Rust 2024](https://img.shields.io/badge/Rust-2024-orange)
![Tokio](https://img.shields.io/badge/runtime-Tokio-blue)
![SQLite WAL](https://img.shields.io/badge/sqlite-WAL-003B57)

`hcodex`는 Gemini 4 Argon, Claude Sonnet/Opus 5.5 Medium, OpenAI Codex를 연결하는 Rust 기반 Tri-Polar 협업 시스템입니다. 하나의 바이너리가 공통 실행 백엔드, CLI, 실시간 TUI, 동적 DAG 오케스트레이터, SQLite 작업 저장소를 제공합니다. hcodex 자체 MCP 서버와 등록 경로는 제거됐습니다.

## 핵심 보장

- 요청 모델과 실제 모델을 영수증에서 분리하며 모델 불일치는 실패로 처리합니다.
- 전송 이후 타임아웃은 `timed_out_unknown`으로 영속화하고 같은 ID로 조회·취소합니다.
- Argon·Claude·Codex 역할 상태와 합의 상태를 분리합니다. 대기·실패·미확정 역할은 합의가 아닙니다.
- DAG는 실제 목표와 소스 구조에 따라 4~8개 노드, 2~4개 wave로 구성합니다. WAW/RAW/WAR 충돌, 순환, 범위 밖 쓰기를 차단합니다.
- layer는 표시용이며, 의존성이 검증된 노드를 빈 슬롯에 즉시 배치합니다. 공유 workspace의 쓰기는 독점 실행합니다. 선행 노드의 검증 영수증과 결과 요약을 후속 노드에 전달합니다.
- 노드는 실제 검증 명령과 SHA-256 매니페스트를 통과해야 `STOPPED`에서 `SETTLED`로 승격됩니다.
- `SETTLED` 노드는 검증 사실과 에이전트의 설명을 분리한 불변 인계 기록을 남깁니다. 후속 노드는 체크포인트 저장을 확인한 뒤 필요한 부모·조상 기록을 전달받습니다.
- `board → council → votes` 흐름은 저장된 작업·인계를 조회하고, 실제 세 역할의 Review Council을 실행하며, 영수증과 제한된 소스 최신성을 함께 판독합니다.
- `research`는 코드·기술 문서·명시 HTTPS 자료를 직접 관측하고, 조사·반론·종합을 거쳐 출처와 연결된 보고서를 만듭니다. 개발 DAG와 별도의 완료 기준을 사용합니다.
- 활성 `SKILL.md` 본문과 관련 소스·CodeGraph 심볼 구간을 실제 디스크에서 읽어 프롬프트에 주입합니다.
- SQL은 `crates/hcodex/sql/`에 분리하며 prepared binding, NUL/stacked-query/meta-command 방어를 적용합니다.
- TUI는 고정 composer/status 영역, 동적 DAG, 실시간 lane transcript, 정확한 Unicode 폭과 증거 단계를 표시합니다.
- `/dag-mode`는 중앙 배지의 진입 shimmer·퇴장 fade를 표시하며, 모드 안의 일반 입력은 DAG로 실행합니다. `/ask`는 직접 질의를 유지합니다. 실행 중 lane만 회전 표시하며 `NO_COLOR` 또는 `TERM=dumb`에서는 정적 배지를 사용합니다.
- DAG 화면은 실제 의존성을 잇는 노드·화살표와 fan-out/fan-in을 표시합니다. 작은 화면에서는 미니 그래프와 부모 pin을 표시하고, 실행 전 `/dag-mode` 화면은 미실행 라우팅 예시로 구분합니다.
- `/skills core` 또는 `/skill core`는 `codex-heterarchy`, `codegraph-operator`, `codebase-refactor-guardian`을 로드합니다. 실제 본문을 확인한 뒤 활성 목록을 바꾸며 직접 질의와 각 DAG 노드에 전달합니다. `/skills default`는 codex 기본 스킬만, `/skills clear`는 선택한 스킬을 해제합니다.

실시간 표시는 실제 노드 상태·검증 단계·경과 시간에 해당합니다. SQLite job·checkpoint가 권위이고 bounded 채널의 TUI 이벤트는 soft-realtime 투영입니다. 느린 소비자로 중간 이벤트를 놓치면 `ProjectionLagged`와 `live_projection.resync_required`를 표시하고 저장된 최종 상태를 다시 읽습니다. provider의 토큰·도구 호출 본문 스트리밍, native Antigravity Teamwork의 전체 lifecycle, OMH의 동적 repair/extend·parked 실행은 아직 제공하지 않습니다. 자세한 경계는 [백엔드 전환](docs/backend-cutover.md)과 [실시간 DAG 점검 기록](docs/realtime-dag-review.md)에 구분해 기록합니다.

## Rust 구성

```text
crates/hcodex/
├── src/
│   ├── runtime.rs          # Codex CLI, Antigravity CLI, HTTP 실행
│   ├── routing.rs          # provider/engine/model 계약과 영수증
│   ├── gateway.rs          # 검증된 backend operation boundary
│   ├── storage/            # jobs/handoffs/CodeGraph SQLite
│   ├── dag/                # 계획, hazard, 실행, checkpoint, settlement
│   ├── context/            # 스킬·관련 소스·CodeGraph 컨텍스트
│   ├── triad/              # pipeline/council/adjudication/decision pack
│   ├── workbench.rs        # 작업판, Review Council 투표, 소스 최신성 판독
│   ├── research/           # 직접 관측한 자료, 주장·인용 무결성, 리서치 보고서
│   ├── tui/                # ratatui/crossterm UI
│   └── lifecycle/          # 설치, 스킬 동기화, runtime identity
└── sql/
```

활성 제품과 검증 코드는 Rust로 구성됩니다. 이전 Python 구현·백업·가상환경·캐시는 완전히 제거했으며, 삭제 명세와 원본 해시·JSON/로그 검증 근거만 보존합니다. 설치 진입점은 `hcodex`입니다. 상세 기록은 `docs/python-retirement-result.md`를 참고하세요.

## 설치

요구사항은 Rustup, 저장소가 선언한 Rust 1.98 toolchain, 로그인된 Codex CLI, 필요 시 실행 중인 Antigravity IDE입니다.

```bash
git clone https://github.com/limhaneul12/casoas.git
cd casoas
cargo install --locked --path crates/hcodex
hcodex setup
hcodex status
```

`hcodex setup`은 설치된 Rust 바이너리 진입점과 Gemini/Codex 스킬을 동기화합니다. hcodex가 과거에 생성한 것으로 정확히 확인되는 MCP 등록과 plugin MCP 파일은 제거하며, 사용자 정의 항목과 다른 MCP 설정은 보존합니다. 상세 절차는 [설치 문서](docs/install.md)를 참조하십시오.

이 checkout을 가리키는 과거 `~/.local/bin/hcodex` Python 스크립트가 있으면 setup/sync가 Rust 바이너리로 연결합니다. 사용자 정의 실행 파일과 기존 심볼릭 링크는 보존합니다.

## CLI와 TUI

```bash
hcodex status
hcodex tui
hcodex tui -c "/dag-plan 저장 계층 리팩토링"
hcodex dag-ulw --plan-only "저장 계층 리팩토링 계획"
hcodex dag-run "저장 계층 리팩토링 실행"
hcodex research --plan-only --document docs/spec.md -- "계약과 한계 조사"
hcodex research-status research-existing-id
hcodex jobs
hcodex inbox
hcodex prune
```

TUI 입력창은 `/`, `$`, `@` 자동완성을 지원합니다. 주요 명령은 `/dag-plan`, `/dag-ulw`, `/dag-run`, `/research`, `/research-status`, `/prompts`, `/back`, `/models`, `/skills`, `/effect`, `/status`, `/diff`, `/review`, `/jobs`, `/clear`, `/quit`입니다.

`Tab`으로 추천을 열고 `↑/↓`로 선택합니다. 추천에는 명령 설명, 모델·권한·스킬 인수가 포함되며 한국어 검색도 지원합니다. `↑/↓` 또는 `Ctrl-P/N`은 추천이 없을 때 입력 기록을 탐색하고, `Ctrl-U/W`는 입력 전체/이전 단어를 지웁니다. `Esc`는 추천 닫기 → 검사 화면 뒤로가기 → 입력 지우기 순서로 동작하며, 종료는 `Ctrl-C` 또는 빈 입력의 `Ctrl-D`입니다. 검사 화면은 `PageUp/PageDown`으로 스크롤합니다.

`@src/lib.rs:220-260`, `@"src/file with spaces.rs":L10-L30`처럼 실제 파일·줄 범위를 첨부할 수 있습니다. `$스킬 /ask 질문`은 스킬 본문과 파일 컨텍스트를 함께 전달합니다. 컨텍스트와 토큰 사용량은 별도 상태 줄에 표시하며, 모델 창 크기가 미보고인 경우 실제 요청 바이트/입력 예산을 표시합니다. `/triad`는 기존 DAG 별칭이고, `/triad-review`는 실제 3극 검토 파이프라인입니다.

조직 작업 흐름은 CLI와 TUI에서 동일합니다.

```bash
hcodex --workspace /path/to/repo board --limit 20
hcodex --workspace /path/to/repo council --file src/lib.rs --file README.md "이 변경을 현재 소스에 반영해도 되는가"
hcodex --workspace /path/to/repo votes cdx-collaboration-id
hcodex --workspace /path/to/repo recovery cdx-campaign-id
hcodex --workspace /path/to/existing/workspace resume cdx-campaign-id
```

TUI에서는 `/board [limit]`, `/council --file relative.rs -- <proposal>`, `/votes <collaboration_id>`를 사용합니다. `board`와 `votes`는 저장된 상태만 읽고 모델을 호출하지 않습니다. `council`은 Argon·Claude·Codex Review Council을 실제로 실행합니다. 파일 범위 없는 `/council <proposal>`도 정책 토론에는 사용할 수 있지만 source freshness는 `unknown`이며 현재 소스 승인으로 쓰지 않습니다. 자세한 판정 규칙과 예시는 [조직 작업판과 Review Council](docs/organization-workbench.md)을 참고하세요.

`recovery <job_id>`와 TUI `/recovery <job_id>`는 기존 작업 기록과 worktree의 현재 관측을 함께 보여줍니다. 스키마 초기화, 오래된 실행 상태 정산, 모델 호출, 삭제 또는 재개를 수행하지 않습니다. 과거 검증·제거 기록과 현재 경로·Git 등록을 구별하며, 관측 결과는 정리나 재개 허가가 아닙니다.

`resume <campaign_id>`와 `/resume <campaign_id>`는 저장된 원래 계획·설정과 검증된 경계를 확인해 이어갑니다. Worktree 재개는 소유권 증거가 있는 기존 작업 공간만 재사용하며 미확정 모델 호출을 다시 보내지 않습니다. `status`와 `board`는 소스·설치 파일·현재 프로세스의 식별 범위를 구별합니다. 자세한 재개 조건은 [DAG worktree 모드](docs/dag-worktree-mode.md)를 따릅니다.

기본 `hcodex`는 상태를 조회하고 `hcodex tui`가 대화형 화면을 엽니다. `dag-ulw`는 `--plan-only`가 없으면 실행합니다. `/use codex-default`는 native Codex 설정으로 기본 모델을 확인하며, 생략/null 모델의 명시적인 역할 기본값과 구분합니다. 전체 Python 기능 추적표는 [패리티 감사](docs/python-rust-parity-audit.md)를 참고하세요.

## 조사·기획·개발 작업 공간

TUI의 **Research & Search · Planning · Building** 공간은 각각 실행하거나 연결할 수
있습니다. `/space research|planning|building` 또는 `Alt+1/2/3`으로 이동하며, 각 공간의
입력 초안·보고서·실행 상태를 유지합니다. 다른 공간의 늦은 결과는 현재 화면을 바꾸지 않습니다.

```text
/research -- 주제 조사
/planning -- 기획 목표
/building --plan <정확한 plan_id>
/workflow --through planning -- 조사하고 기획할 목표
/workflow --stages planning,building -- 기획하고 개발할 목표
/workflow --through building -- 조사·기획·개발할 목표
/workflow-status <flow_id>
```

조사·기획은 원본 저장소 밖의 별도 실행 공간에서 진행합니다. 개발을 명시한 요청만,
기획의 원본 manifest와 현재 소스가 일치하고 원본 Git이 staged·unstaged·untracked 모두
clean할 때 기존 에이전트별 worktree DAG로 이어집니다. 차단되면 앞선 산출물을 보존합니다.
기획의 `BuildReady`는 기계적 실행 조건이며 의미상 승인이나 테스트 성공을 뜻하지 않습니다.
기존 `/plan`과 개발 DAG 명령의 의미는 유지합니다. 자세한 실행·인계·복원·취소 계약은
[작업 공간 안내](docs/workspaces.md)를 참고하세요.

## 리서치 모드

```bash
hcodex --workspace /path/to/repo research \
  --file src/lib.rs --document docs/spec.md \
  --url https://example.com/spec \
  --request-id research-contracts -- "구현과 문서의 보장 범위를 비교하라"
hcodex --workspace /path/to/repo research -- "Rust 2024 마이그레이션 변경점을 조사하라"
hcodex --workspace /path/to/repo research-status research-contracts
```

TUI는 `/research [옵션] -- 질문`과 `/research-status <request_id>`를 사용합니다.
질문만 주면 bounded native web discovery로 후보 URL을 찾고, hcodex가 공개 HTTPS 본문을
직접 수집해 확인한 자료만 조사·반론·종합의 근거로 사용합니다. 검색 결과의 snippet이나
모델이 제안한 URL 자체는 근거가 아닙니다. 명시한 파일·문서·URL만 사용하려면 `--no-web`을,
명시 자료와 검색 자료를 함께 쓰려면 `--web`을 지정합니다. `--plan-only`는 검색·수집·모델·DB
효과 없이 계획만 표시합니다. 사이트 크롤링·PDF 처리와 현재 자료를 이용한 과거 시점 증명은
아직 지원하지 않습니다.

조사자 1~3개와 challenger·synthesizer 모델은 요청별로 지정할 수 있습니다.
검색 예산의 `max_search_events`는 hcodex가 실제 관측한 native web-search 이벤트 수를
제한하며 provider 내부의 검색어 묶음 수를 뜻하지 않습니다. raw 검색 로그는 별도 보관하지
않습니다. 다만 Codex CLI 자체 세션 보존 정책은 hcodex DB·보고서와 별도입니다. 보고서는
host가 직접 수집한 source/excerpt ID와 연결된 주장으로 렌더링하며,
미확정·반박된 내용은 별도 기록으로 보존합니다. 인용 무결성 검증은 의미상 진실 판정과
구분하고 `NOT_MECHANICALLY_PROVEN` 경계를 표시합니다. 개발 DAG의 빌드·테스트
`SETTLED` 기준과 Review Council은 바꾸지 않습니다.

공개 `crates/hcodex/assets/research-rules.md`와 고정 연구 corpus는
`crates/hcodex/research-assets.toml`의 SHA-256로 검증합니다. Corpus 검사는 runtime과 같은
Rust validator를 사용하며 canonical CI에서는 웹·모델·LLM judge를 호출하지 않습니다.
자세한 입력·취소·저장 계약은 [리서치 모드](docs/research-mode.md),
설계 근거는 [업스트림 벤치마크](docs/research-reference-benchmark.md)를 참고하세요.

## 백엔드 경계

CLI와 TUI는 같은 `Gateway` application boundary를 직접 호출합니다. 모델 실행, 작업 조회, 핸드오프, Triad, DAG와 상태 판독은 closed input, 구조화 출력 검증, 실제 실행 영수증, evidence scope를 사용합니다. caller가 적은 테스트 명령은 실행 증거가 아니며 hcodex가 직접 실행한 게이트만 `test_execution=verified`가 될 수 있습니다. 외부 network server나 MCP transport는 제공하지 않습니다.

## 데이터와 복구

한 파일 복구 관찰 실험은 `.hcodex/changes/<change_id>/before`와 `after`에
원본·예정 결과를 보관하며, 기존 작업 DB가 불변 ID·경로·해시·길이의 권위입니다.
재시작 뒤 원본 상태·예정 결과 상태(미검증)·충돌을 관찰하며 자동 적용·복원·재실행은
하지 않습니다. 현재 DAG 실행 경로에는 연결되지 않은 라이브러리 실험입니다.
검증 범위는 [복구 실험 기록](docs/workspace-recovery-experiment.md)에 있습니다.

에이전트별 독립 Git worktree 실행은 명시적 모드로 선택합니다.

```bash
hcodex dag-run --worktree "저장 계층 검토"
```

이름 있는 작업 공간은 `hcodex --worktree-name feature-auth`로 생성하고 TUI를 엽니다.
같은 이름으로 다시 실행하면 원래 작업 공간을 보존한 채 다시 엽니다. 이름은 영문·숫자·
`-`·`_`로 1~64자이며, 생성/열기 시 원본 저장소가 untracked까지 포함해 clean해야 합니다.
이 세션에서 `/dag-mode`를 켜면 각 노드의 독립 worktree 모드가 기본 적용됩니다.
계획만 보려면 `hcodex --worktree-name feature-auth dag-ulw --plan-only "목표"`를 사용합니다.

`1 → 2 → 3` 의존 관계에서는 1의 검증된 전체 소스 변경과 새 파일이 2의 baseline에,
2의 변경까지 반영된 전체 상태가 3의 baseline에 전달됩니다. 실패·미확정 결과는 전달하지
않습니다. 각 `SETTLED` 노드는 실제 모델·gate·매니페스트·변경 경로와 제한된 결정·주의사항을
체크포인트에 함께 저장합니다. hcodex가 해시와 범위를 확인해 후속 프롬프트를 구성하므로,
조회 도구가 없는 모델도 필요한 조상 이력을 받습니다. 필수 기록이 없거나 예산을 넘으면
`context_not_ready`로 중단하며 완전한 인계를 가장하지 않습니다. 노드 worktree는 검증·통합 후 정리하며, 최종 변경을 모은 이름 있는 작업 공간은
사용자가 검토할 수 있도록 보존합니다. 원래 저장소에는 자동 반영·커밋하지 않습니다.

TUI에서는 `/dag-run --worktree <goal>`, native CLI에서는 `hcodex dag-run --worktree <goal>`을
사용합니다. 생략하면 기존 direct/serial 동작을 유지하며, worktree 모드 실패를 direct로
자동 전환하지 않습니다. 원본 작업 폴더의 staged·unstaged·untracked 상태가 모두 clean해야
시작하며, 기준 커밋만 clean하거나 dirty 파일을 별도로 복사하는 방식은 허용하지 않습니다.
모든 노드의 detached worktree는 원본이 clean한 시작 시점에 미리 생성하고, 선행 노드의
검증된 결과는 이미 만든 후속 worktree에 반영합니다. 원본 결과 반영은 직렬화됩니다.

노드 검증과 원본 반영 후 검증, checkpoint·campaign 결과 저장 및 소유 프로세스 종료가
확인된 작업만 정리합니다. 미확정·실패·예상 밖 파일·정리 오류는 경로와 근거를 보존하며
사용자가 만든 branch/worktree를 삭제하지 않습니다. 보존 경로와 상태는 DAG 결과의
worktree 보고서에서 확인합니다. worktree는 파일 작업 공간 분리이며 네트워크·인증·공유
Git 상태의 보안 sandbox나 속도 향상을 보장하지 않습니다.

기본 상태는 `~/.cache/codex-collab-mcp/` 아래의 Python 역사 데이터와 호환되는 SQLite DB에 저장됩니다. `request_id`/`idempotency_key`는 재시작 뒤에도 유지합니다. 완료·실패·취소·미확정 상태를 서로 덮어쓰지 않습니다. 동일 체크포인트의 정확한 재생은 기존 기록을 바꾸지 않으며, 같은 ID의 다른 범위·내용은 충돌로 거절합니다. 완료·실패·취소·미확정 작업은 일반 retry 플래그로 다시 실행하지 않고 기존 ID로 조회합니다.

프로세스와 async task는 bounded supervisor가 소유합니다. 취소·타임아웃·종료 시 하위 프로세스 그룹을 종료하고 reap합니다. broad `pgrep`로 다른 세션을 종료하지 않습니다.

## 개발 규칙과 검증

Rust 작업은 `AGENTS.md`와 로컬에 materialize한 `.agents/rust_dev_harness/`의 규칙을 따릅니다.
`.agents/`와 `.cargo/`는 로컬 전용이며 Git에 포함하지 않습니다. canonical gate는 다음 하나입니다.

```bash
make ci
```

이 명령은 공개 `xtask/HARNESS.toml`의 기계적 정책, 프로젝트 연구 자산 SHA-256 무결성,
`cargo fmt --check`, workspace/all-target/all-feature check, Clippy `-D warnings`, 전체 테스트와
실제 Rust CLI/TUI 경계 fixture를 실행합니다. 로컬 `.cargo` alias와 개인 개발 하네스 없이도
같은 공개 검증을 실행합니다.

로컬에 materialize한 Rust/shared 개발 규칙의 전체 파일·해시 무결성은 별도로 확인합니다.

```bash
make local-harness
```

이 명령은 로컬 bundle이 없거나 변경되면 실패합니다. 해당 로컬 검증은 공개 CI에 포함되거나
클린 체크아웃에서 실행됐다고 간주하지 않습니다.

프로세스 경계 테스트의 병렬 실행은 네 개로 제한합니다. 타임아웃·모델 identity 검증을 완화하지 않고 호스트 CPU/파일시스템 경합을 줄이는 canonical CI 자원 예산입니다.

기존 데이터 형식은 Python 저장소가 생성한 고정 SQLite fixture로 검증합니다. 이전 Python 참조본은 활성 런타임이나 CI에서 실행하지 않습니다.

## 상태 판독

- `succeeded`/`confirmed_success`: 실행과 구조화 출력 검증이 완료됨
- `failed`/`confirmed_failure`: 실행 또는 검증 실패가 확인됨
- `rejected_pre_dispatch`: provider/engine/model 계약 위반으로 실행 전 거부됨
- `timed_out_unknown`/`outcome_unknown`: provider가 요청을 받았을 가능성이 있어 결과를 단정할 수 없음
- `queued`: 핸드오프 또는 백그라운드 작업이 대기 중이며 완료 증거가 아님

미확정 작업은 새 ID로 재실행하지 말고 `hcodex jobs`, `hcodex recovery <job_id>` 또는 TUI의 대응 명령으로 기존 ID를 조회하십시오.
