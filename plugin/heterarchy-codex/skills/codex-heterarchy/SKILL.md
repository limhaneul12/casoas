---
name: codex-heterarchy
description: Rust hcodex CLI·TUI와 공통 백엔드로 Argon·Claude Medium·Codex의 Tri-Polar 협업, 동적 DAG·노드 인계·리서치 보고서·조직 작업판·Review Council을 운영하고 실제 모델 영수증·소스 최신성·미확정 결과를 판독한다.
---

# Rust Heterarchy Codex

이 스킬은 Rust `hcodex` CLI·TUI가 직접 호출하는 공통 백엔드로 Argon의
거시 설계, Claude의 정밀 구현, Codex의 적대적 감사를 결합한다. hcodex
자체 MCP 서버나 별도 네트워크 서비스를 전제로 하지 않는다.

## 시작 전 확인

1. 실제 선택된 `hcodex --version`, `hcodex --help`, `hcodex status`를 확인한다.
2. 대상 workspace와 파일·심볼을 명시한다. 생략 시 Rust relevance/CodeGraph
   탐색 결과를 확인한다.
3. write 작업은 DAG의 declared `write_files`, preimage SHA-256, 검증 명령을
   고정한다.
4. provider timeout은 원래 ID로 조회한다. 다른 ID로 자동 재시도하지 않는다.

## 절대 판정 규칙

- `requested_model != actual_model`이면 성공으로 취급하지 않는다.
- `rejected_pre_dispatch`는 provider 호출이 발생하지 않은 확정 실패다.
- `timed_out_unknown`은 성공도 실패도 아니다. `hcodex jobs`와
  `hcodex recovery <job_id>` 또는 TUI 대응 명령으로 원래 ID를 조회한다.
- 핸드오프 `queued`는 실행 완료나 합의가 아니다.
- 필수 Argon·Claude·Codex 역할 모두가 실제 성공 영수증, 비어 있지 않은
  검토 artifact, 명시적 `PASS|WARN|BLOCK`를 가져야 합의를 계산한다.
- `BLOCK`은 승인으로 바꾸지 않는다. 모든 역할이 `WARN`이어도
  `review_approved=false`다.
- caller가 전달한 테스트 명령은 실행 증거가 아니다. hcodex가 직접 실행한
  성공 영수증만 verified다.

## 권장 라우팅

- 아키텍처·최종 심의: `gemini-4-argon-high`
- 구현: `claude-sonnet-5-5-medium` 또는 `claude-opus-5-5-medium`
- 감사·합성: `gpt-6-terra|luna|sol|astra`, `gpt-6.1-terra|sol`
- Claude low/high tier, GPT 5.x, provider/engine mismatch는 실행 전에 거부한다.

실제 제공 모델은 매번 `hcodex models` 또는 TUI `/models`로 확인한다.

## 실행 표면

```bash
hcodex --workspace /path/to/repo status
hcodex --workspace /path/to/repo argon "아키텍처 검토"
hcodex --workspace /path/to/repo sonnet "구현 검토"
hcodex --workspace /path/to/repo opus "심층 구현 검토"
hcodex --workspace /path/to/repo dag-ulw --plan-only "목표"
hcodex --workspace /path/to/repo dag-run "목표"
```

Codex 기본 모델을 직접 사용할 때는 TUI에서 `/use codex-default` 후 `/ask`를
사용한다. 설치본의 help에 없는 명령을 추정해서 호출하지 않는다.

## DAG-ULW++

- plan-only로 topology와 WAW/RAW/WAR/cycle/path hazard를 먼저 확인한다.
- 검증된 의존성이 충족된 노드를 빈 슬롯에 즉시 배치하며 공유 workspace의
  쓰기는 독점 실행한다.
- 후속 노드는 선행 노드의 실제 소스 상태, 검증 영수증과 제한된 인계 기록을
  받는다. 요약문만으로 소스 전달을 대신하지 않는다.
- TUI는 노드·provider·검증·settlement를 soft-realtime으로 표시한다. SQLite
  job/checkpoint가 권위이고, 중간 화면 이벤트가 지연되면 `ProjectionLagged`와
  `live_projection.resync_required`를 확인한 뒤 저장 상태를 다시 읽는다.
- provider 완료는 `STOPPED`다. 실제 gate와 전체 manifest 검증 뒤에만
  `SETTLED`다.
- Argon/Claude가 child-agent 실행을 서술해도 독립 도구 영수증이 없으면
  별도 에이전트 완료로 세지 않는다.

## 이름 있는 세션과 노드 worktree

```bash
hcodex --worktree-name feature-auth
hcodex --worktree-name feature-auth dag-ulw --plan-only "목표"
hcodex dag-run --worktree "목표"
```

- 새 session/DAG 생성 전 원본 작업 폴더의 staged·unstaged·untracked 상태가
  모두 clean해야 한다. stash, reset, 자동 commit이나 dirty 복사로 우회하지
  않는다.
- 모든 pending 노드 worktree를 clean 시작점에 미리 생성한다.
- `1 → 2 → 3`에서는 1의 검증된 수정·새 파일이 2의 baseline에, 2의 변경까지
  누적된 전체 상태가 3의 baseline에 전달된다.
- canonical 반영은 직렬화하고 isolated gate와 반영 대상 gate·manifest가 모두
  통과해야 `SETTLED`다.
- 체크포인트, 결과 export, 소유 process group 종료와 정확한 작업 공간 증거가
  확인된 노드만 정리한다. unknown·drift·부분 정리는 경로와 원래 ID를 보존한다.
- 최종 변경이 모인 이름 있는 session은 자동 삭제하지 않는다.

## 재개와 복구

```bash
hcodex --workspace /path/to/existing/workspace resume cdx-campaign-id
hcodex --workspace /path/to/repo recovery cdx-job-or-campaign-id
```

TUI는 `/resume <campaign_id>`, `/recovery <job_id>`를 사용한다. Resume은 저장된
정확한 계획·모델·스킬 본문·설정과 verified frontier를 사용한다. 미확정 provider를
새 ID로 보내지 않는다. 준비·admission·부분 반영·소유권 모순이 있으면 보존 후
거절한다. Recovery는 조회 전용이며 DB 초기화, 모델 호출, 삭제, 재전송이나 재개
권한을 만들지 않는다.

## 조직 작업판과 Review Council

```bash
hcodex --workspace /path/to/repo board --limit 20
hcodex --workspace /path/to/repo council --file src/lib.rs --file README.md "검토 제안"
hcodex --workspace /path/to/repo votes cdx-collaboration-id
```

- `board`와 `votes`는 저장 상태만 읽고 모델을 호출하지 않는다.
- `council`은 실제 세 역할 검토를 실행한다. queued/running 역할은 투표가 아니다.
- 파일 범위 없는 council은 정책 토론에는 사용할 수 있지만 source freshness는
  `unknown`이며 현재 코드 승인 근거가 아니다.
- `current_scoped`는 열거한 파일의 현재 SHA-256만 판독한다. 전체 workspace,
  provider E2E, native UI나 배포 검증이 아니다.

## TUI 조사·기획·개발 작업 공간

```text
/space research
/space planning
/space building
/research -- 조사할 질문
/planning --from <research_id> -- 기획 목표
/building --plan <plan_id>
/workflow --through planning -- 조사하고 기획
/workflow --stages planning,building -- 기획하고 개발
/workflow --through building -- 조사·기획·개발
/workflow-status <flow_id>
/workflow-cancel <flow_id>
```

- 세 공간은 독립 실행하거나 명시한 순서로 연결한다. 기본 `/workflow`는 조사·기획까지며,
  Building은 명시적으로 포함한 요청에만 실행한다. 기존 `/plan`·`/dag-plan` 의미는 유지한다.
- `Alt+1/2/3`으로 공간을 바꿀 수 있다. Research·Planning 공간의 일반 입력은 해당 작업으로
  보내며 DAG mode보다 우선한다. Building의 기존 입력 동작과 숫자 단축키는 유지한다.
- 선택한 공간은 함께 예약한다. 실행 ID가 다른 늦은 결과가 다른 공간이나 새 작업을 덮지 않는다.
- 조사·기획 모델 cwd와 상태 저장소는 원본 저장소 밖이다. 출처는 실제 원본 파일에 묶는다.
- Planning은 실제 모델의 닫힌 proposal을 저장한다. `BuildReady`는 schema·source/reference·path·
  dependency·지원 gate·DAG 조건을 검사한 상태이며 의미상 승인이나 실행 검증이 아니다.
- Building은 정확한 저장 plan ID/digest와 원본 source manifest를 다시 확인하고, 원본이 staged·
  unstaged·untracked 모두 clean일 때 기존 node worktree 경계로 들어간다. 조사/기획을 보존하며
  dirty를 stash/reset/자동 commit으로 우회하지 않는다. 현재 소스가 달라지면 기획을 다시 만든다.
- 후속 단계에는 요약만이 아니라 저장된 조사 근거·기획 범위·완료 조건과 실제 source/handoff를
  전달한다. 알려진 incomplete evidence는 기획에 공백을 남겨 전달하지만 unknown/cancelled는 멈춘다.
- 동일 flow ID는 재실행하지 않는다. `/workflow-status`는 원래 산출물을 조회·복원하며 초안이나
  활성 탭의 재시작 복원을 약속하지 않는다. 취소는 현재 소유 Gateway에서만 신호·회수를 수행한다.

## 리서치 모드

개발 DAG와 별도로 코드·로컬 기술 문서·공개 HTTPS 자료를 조사한다. 질문만
주면 한 번의 제한된 native Codex 웹 탐색으로 후보 URL을 찾고, host가 본문을
직접 수집·검증한 자료만 조사·반론·종합 단계의 근거로 사용한다.

```bash
hcodex --workspace /path/to/repo research -- "질문만으로 공개 자료 조사"
hcodex --workspace /path/to/repo research --file src/lib.rs --document docs/spec.md --url https://example.com/spec --request-id research-contracts -- "계약과 한계 비교"
hcodex --workspace /path/to/repo research --web --file src/lib.rs -- "코드와 최신 공개 자료 비교"
hcodex --workspace /path/to/repo research --no-web -- "출처 없이 계획만 확인"
hcodex --workspace /path/to/repo research-status research-contracts
```

TUI는 `/research [옵션] -- 질문`, `/research-status <request_id>`를 사용한다.
TUI 경로에 공백이 있으면 native CLI의 인용된 옵션을 사용한다.
`--plan-only`는 검색 의도를 포함한 계획만 반환하며 source fetch, provider 호출,
DB 초기화를 하지 않는다. 질문만 있는 요청은 기본 웹 탐색을 사용한다. 명시한
출처만 사용하려면 `--no-web`을 선택하며, 이때 출처 없는 요청은 누락 입력 계획을
반환한다. 크롤링·PDF 파싱·과거 시점 증명은 제공하지 않는다.

- 웹 탐색은 기본 `gpt-6.1-sol/high`, 관측 가능한 `web_search` 이벤트 최대 3개,
  후보 URL 6개, host가 받아들인 source 최소 2개, 60초 제한을 사용한다. 하드
  한도는 이벤트 6개와 후보 12개다. 이벤트 수는 provider 내부 query 확장 수를
  증명하지 않는다.
- 명시 출처가 있는 요청은 기본적으로 검색하지 않는다. 같은 요청에서 공개 자료도
  찾으려면 `--web`을 사용하고, 지원되는 경우 `--search-model`로 탐색 모델을
  지정한다.
- 탐색은 빈 임시 디렉터리와 read-only sandbox에서 실행하고 알려진 shell·파일
  실행·MCP·multi-agent 기능 경로를 끈다. 정확한 native `web_search` 완료 이벤트,
  실제 모델 identity와 소유 process group 종료를 확인할 수 없으면 분석을 시작하지
  않는다. 예상하지 않은 tool 이벤트도 원래 탐색 ID 아래 실패로 보존한다.
- 후보 URL은 완료된 `web_search` 이벤트의 `results[].url`에 실제 포함된 주소와
  정규화 후 일치해야 한다. 자유 형식 model 출력은 후보를 추가할 수 없다.
  hcodex DB·보고서는 검색 query·snippet·reasoning을 복사하거나 citation으로
  사용하지 않는다. native Codex session은 provider 자체 정책에 따라 실행 증거를
  보존할 수 있다. 후보는 host의 DNS/IP·redirect 검증과 본문 수집을 통과해
  source/excerpt 영수증이 생긴 뒤에만 evidence가 된다.
- 발견한 후보의 수집 실패는 optional rejection으로 남는다. 사용자가 명시한 출처의
  실패는 계속 required failure이며, 설정된 최소 accepted source를 충족하지 못하면
  investigator를 호출하지 않는다.
- 탐색 ID는 parent와 versioned effective 설정에 묶이고 완료 영수증과 canonical
  후보를 fetch 전에 저장한다. 같은 request ID는 검색·수집·분석을 반복하지 않으며
  timeout/unknown을 새 ID로 자동 재전송하지 않는다.

- `--file`·`--document`는 workspace 상대 경로, `--url`은 명시 HTTPS source다.
- 실제 source 관측 시각·본문 해시·제한된 excerpt와 위치를 host가 기록한다.
  모델이 말한 URL은 관측 source가 아니다.
- `--investigator MODEL`은 1~3개 조사 lane을 지정하며, challenger와
  synthesizer도 선택한다. 같은 모델을 쓰더라도 서로 다른 실제 job ID가 필요하다.
- 보고서는 source/excerpt ID가 연결된 claim으로 host가 렌더링한다.
  미확정·반박·상충 근거는 보존하고, 자유 형식 advisory를 검증된 본문으로 바꾸지 않는다.
- 인용·해시·참조 무결성과 의미상 진실은 별개다. 후자는
  `NOT_MECHANICALLY_PROVEN`이며 표 수나 agent 동의로 승격하지 않는다.
- 동일 request ID는 원래 자료·단계 기록 조회용이다. timeout/unknown을 새 ID로
  자동 재전송하지 않는다. 다른 자료를 조사하려면 새 요청을 만든다.
- CLI Ctrl-C와 같은 TUI 소유 Gateway의 종료는 admitted work를 회수하고
  제한된 partial 보고서와 단계 ID를 보존한다. 다른 프로세스의 status 조회는 취소가 아니다.
- 리서치 완료는 코드 빌드·테스트 증거 또는 개발 DAG `SETTLED`가 아니다.
  공개 `crates/hcodex/tests/fixtures/research_contract_cases.json` corpus는 runtime과 같은 validator로 검사하며
  canonical CI에서 웹·모델·사람 평가를 요구하지 않는다.

## Native Antigravity 경계

신규 Argon/Claude 작업은 Antigravity CLI JSON stream으로 실행한다. 과거 RPC
기록은 원래 ID로 읽는다. `AGY_CLI_BIN`, PATH의 `agy`, 관리되는 libexec 순으로
CLI를 찾는다. CLI가 보고한 모델은 설정 관측이며 독립 서버 모델 증명은 아니다.

hcodex는 첫 입력 전에 세션 ID를 SQLite에 확정하고, 실행 중에는 단계와 사용량 등
제한된 진행 정보만 저장한다. 기본 실행 데드라인은 없고 양수로 지정할 수 있다.
시작·입출력·정리의 별도 제한은 유지한다. 상태 조회는 읽기만 한다. CLI 대화
후속 요청은 새 턴이며, 연결을 잃은 실행 중 턴에 재접속하는 기능은 없다.

격리된 디렉터리에서 결과물만 받아 호스트가 검증하고 적용한다. 검증된 CLI의
strict sandbox 정책으로 파일 쓰기와 셸 명령을 차단한다. 미검증 CLI가 도구를
노출하면 입력 전에 거절한다. Parent `SUCCESS`를 팀 완료로 세지 않으며 native
Teamwork 완료나 전체 팀 취소를 현재 기능으로 광고하지 않는다.

## 설치와 검증

```bash
cargo install --locked --path crates/hcodex
hcodex setup
hcodex status
hcodex tui
```

`setup`/`sync`는 launcher와 Gemini/Codex skill을 동기화하고 정확히 식별된 과거
hcodex MCP 등록만 제거한다. 사용자 정의 설정과 다른 MCP 항목은 보존한다.
`mcp`, `on`, `off`, `restart`는 제거된 명령이다.

Rust 개발은 `AGENTS.md`와 `.agents/rust_dev_harness/`를 따르고 canonical
`make ci`를 실행한다. 실행하지 않은 검증은 PASS로 보고하지 않는다.
