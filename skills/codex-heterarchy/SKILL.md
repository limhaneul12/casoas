---
name: codex-heterarchy
description: 제미나이 4 아르곤(Gemini 4 Argon), 클로드 소넷 5.5 / 오퍼스 5.5(Claude Sonnet 5.5 / Opus 5.5 Medium), OpenAI Codex(GPT) 간의 3극(Tri-Polar) 헤테라키 협업 및 검증 기반 오케스트레이션을 수행할 때 사용한다. 설치 및 MCP/스킬 등록(hcodex), 검증된 라우팅 계약(provider/engine/model), 실행 영수증(receipt), 조회 가능한 타임아웃(timed_out_unknown), 역할별 협업 상태(complete/partial)와 합의 상태(consensus_verified/incomplete_no_consensus) 분리, 증거 범위(CodeGraph/SHA-256/test_execution) 고정 사용법을 포함한다.
---

# `codex-collab-mcp` — 3극(Tri-Polar) 헤테라키 오케스트레이션 스킬 및 사용 가이드

이 스킬은 **Gemini 4 Argon (거시 아키텍트·롱컨텍스트 조망)**, **Claude Sonnet 5.5 / Opus 5.5 Medium (외과수술식 코드 구현·심층 아키텍처 리팩토링)**, 그리고 **OpenAI Codex (적대적 레드팀 감사·고밀도 알고리즘 합성·격리 디버깅)**를 `codex-collab-mcp` 브리지로 연결하여 협업하는 **검증 기반(Receipt-Verified) 실행 규약 및 사용 설명서**다.

---

## 0. 빠른 시작 (설치 및 스킬·MCP 자동 등록)

### 사전 요구사항
- **OS**: macOS (또는 POSIX 호환 환경)
- **패키지 매니저**: [`uv`](https://docs.astral.sh/uv/) (Python `3.14` Free-Threaded 런타임 자동 관리)
- **외부 런타임**:
  - **OpenAI Codex CLI** (`codex` 바이너리가 `PATH` 또는 `/opt/homebrew/bin/codex`에 설치 및 로그인된 상태)
  - **Google Antigravity IDE** (`claude_code_craft` 및 `heterarchy_triad_orchestrate` 사용 시 로컬 LanguageServer RPC 자동 탐색)

### 1분 설치 및 원클릭 등록 (`hcodex`)
```bash
# 1. 저장소 클론 및 가상환경 동기화
git clone https://github.com/limhaneul12/casoas.git
cd casoas
uv sync

# 2. Antigravity & Codex 글로벌 스킬 등록 + MCP 설정 자동 활성화
uv run hcodex on

# 3. 연결 및 런타임 상태 확인
uv run hcodex status
```

`uv run hcodex on`을 실행하면 현재 클론된 경로를 기준으로 다음 설정이 **자동 생성·동기화**된다:
1. **Antigravity 글로벌 스킬**: `~/.gemini/config/skills/codex-heterarchy/SKILL.md`
2. **Codex 글로벌 스킬**: `~/.codex/skills/codex-heterarchy/SKILL.md`
3. **Antigravity 플러그인 MCP 설정**: `~/.gemini/config/plugins/heterarchy-codex/mcp_config.json`
4. **Codex MCP 설정**: `~/.codex/config.toml` (`[mcp_servers.heterarchy_codex]`)

### 수동 MCP 등록 방법 (직접 설정 파일에 추가할 경우)
다른 MCP 클라이언트나 설정 파일에 직접 등록할 때는 `<REPO_PATH>`를 클론한 절대 경로로 치환하여 등록한다:

```json
{
  "mcpServers": {
    "heterarchy-codex": {
      "command": "<REPO_PATH>/.venv/bin/python3",
      "args": ["-m", "codex_collab_mcp.server"],
      "env": {
        "PYTHONPATH": "<REPO_PATH>/src"
      }
    }
  }
}
```

---

## 1. 핵심 보장 원칙

- **라우팅 가능 여부 사전 검증**: 모든 호출과 오케스트레이션은 **`codex_bridge_health`의 실제 라우팅 가능 여부(`routable_*`)**와 **실행 영수증(`receipt`)이 확인된 범위에서만** 완료로 인정한다.
- **조용한 Fallback 금지**: 지원하지 않는 `(provider, engine, model)` 조합은 실제 디스패치 전에 `status="rejected_pre_dispatch"`로 즉시 거부하며, 다른 CLI나 기본 모델로 몰래 대체하지 않는다.
- **타임아웃 상태 보존 (`timed_out_unknown`)**: 응답 유실 가능 타임아웃은 실패나 성공으로 단정하지 않고 `status="timed_out_unknown"` (`outcome_state="outcome_unknown"`)으로 영속화하며, 동일 `request_id`/`idempotency_key`로 조회·취소한다.
- **핸드오프 티켓과 실행 완료 분리**: 핸드오프 티켓 발행(`heterarchy_handoff_create`)은 `collaboration_role_state="queued"`(`status="pending"`)일 뿐 전달·실행·완료 증거가 아니다.
- **엄격한 3극 합의 판정**: 모든 필수 역할(`argon`, `sonnet`/`opus`, `codex`)이 `state="succeeded"`이고 명시적 판정(`PASS`/`WARN`/`BLOCK`)과 검토 요약을 반환하지 않으면 절대 3극 합의(`consensus_reached=true`)나 전체 완료(`collaboration_status="complete"`)로 보고하지 않는다.

---

## 2. 제공되는 MCP 도구 일람 (15개 Tool Selector)

| 목적 | MCP 도구 이름 | 검증된 라우팅 계약 (`provider` / `engine`) | 설명 |
| :--- | :--- | :--- | :--- |
| **🔥 3극 통합 오케스트레이션** | `heterarchy_triad_orchestrate` | `antigravity_local_rpc` + `codex_cli` | `pipeline`(소넷/오퍼스 구현 → 코덱스 감사 → 아르곤 심의) 또는 `council`(병렬 심의) 수행 (`claude_model="claude-sonnet-5-5-medium"` 또는 `"claude-opus-5-5-medium"`, 부분 실패 이어달리기 `resume_collaboration_id`, 다회차 자동 반론·수정 루프 `max_rounds`, 브리지 직접 테스트 실행 `run_verification_gates=True` 지원) |
| **✨ 클로드 소넷/오퍼스 5.5 정밀 구현** | `claude_code_craft` | `antigravity` / `antigravity_local_rpc` (`claude-sonnet-5-5-medium` / `claude-opus-5-5-medium`) | 로컬 Antigravity RPC로 Claude Sonnet 5.5 또는 Opus 5.5 Medium 호출 (`opus-5.5` 별칭 지원, Medium 외 티어 요청 시 `CLAUDE_TIER_POLICY_VIOLATION` 거부) |
| **🛡️ 적대적 코드 감사 (레드팀)** | `codex_adversarial_audit` | `openai_codex` / `codex_cli` (또는 명시적 `antigravity_local_rpc`) | 코드/Diff와 설계 의도를 검증해 `PASS`/`WARN`/`BLOCK` 판정, 실패 재현 테스트, `receipt`, `evidence_scope` 반환 |
| **⚡ 초고밀도 핀포인트 구현** | `codex_targeted_synthesize` | `openai_codex` / `codex_cli` | 시그니처와 계약을 넘겨 구현 코드와 유닛 테스트 합성 (`test_generation`과 `test_execution` 증거 분리 기록) |
| **⚖️ 다중 트레이드오프 대안 탐색** | `codex_differential_implementations` | `openai_codex` / `codex_cli` | 최적화 축별 구현 후보군(2~3개)과 트레이드오프 비교 가이드 반환 |
| **🚑 격리 디버깅 구원 (Rescue)** | `codex_rescue_sandbox` | `openai_codex` / `codex_cli` | 반복 실패한 버그/테스트 트레이스를 넘겨 근본 원인 분석 및 패치 도출 (`async_background=True` 지원) |
| **🔍 네이티브 Git 리뷰** | `codex_git_review` | `openai_codex` / `codex_cli` | 로컬 Git 변경사항(`--uncommitted`, `--base`, `--commit`)에 대해 `codex review` 실행 및 영수증 반환 |
| **🔄 다회차 세션 이어가기/분기** | `codex_session_followup` | `openai_codex` / `codex_cli` | 이전 실행의 `session_id`를 받아 `resume` 또는 `fork` 실행 |
| **⏳ 잡/타임아웃 상태 조회 및 취소** | `codex_job_status` / `codex_job_list` | 공통 (`job_id` / `idempotency_key` / `cancel`) | 비동기 작업 및 `timed_out_unknown` 요청의 상태·영수증 조회, 안전한 취소 요청(`cancel=True`) 수행 |
| **📬 3극 작업 티켓 발행/조회/완료** | `heterarchy_handoff_create` / `inbox` / `resolve` | `argon` ↔ `sonnet` ↔ `codex` | 역할 간 비동기 작업 위임 티켓 생성, 인박스 조회 및 처리 상태 업데이트 |
| **🏛️ 합의안/결정문 패키징** | `codex_alexandria_decision_pack` | 공통 | 협업 결과를 SHA-256 멱등 키가 부여된 마크다운 결정문으로 패키징 (`collaboration_id` 검증 지원) |
| **🩺 브릿지 & 라우팅/인증 헬스체크** | `codex_bridge_health` | 공통 | 광고 모델 vs 실제 라우팅 가능 모델(`routable_*`), 검증 시각(`verified_at_utc`), 독립된 인증 경계(`auth_boundaries`) 확인 |

---

## 3. 주요 워크플로우별 호출 가이드

### 워크플로우 A: 3극 통합 파이프라인 (`heterarchy_triad_orchestrate`)
Claude가 코드를 작성/리팩토링하고, Codex가 레드팀 감사를 수행한 뒤, Argon이 최종 심의하는 파이프라인:
```python
heterarchy_triad_orchestrate(
    task_description="비동기 레이트 리미터의 동시성 경합 조건을 수정하고 경계 테스트를 추가하라.",
    workspace_dir="/path/to/workspace",
    mode="pipeline",  # "pipeline" 또는 병렬 심의 "council"
    claude_model="claude-sonnet-5-5-medium",  # 또는 "claude-opus-5-5-medium"
    codex_model="gpt-5.4",
    max_rounds=2,  # Codex가 WARN/BLOCK 판정 시 자동 수정·재감사 루프 (1~3)
    run_verification_gates=True,  # 브리지가 직접 테스트 게이트 실행
    verification_commands=["pytest -q"],
)
```

### 워크플로우 B: 적대적 레드팀 코드 감사 (`codex_adversarial_audit`)
중요한 비즈니스 로직이나 보안/동시성 변경사항에 대해 Codex 레드팀 감사를 단독 호출:
```python
codex_adversarial_audit(
    code_or_diff="<검토할 코드 또는 git diff>",
    argon_design_rationale="토큰 버킷 상태 갱신 시 락 경합 없이 원자성을 보장해야 함",
    workspace_dir="/path/to/workspace",
    focus_areas=["concurrency", "edge_cases", "security"],
)
```

### 워크플로우 C: 외과수술식 코드 구현 (`claude_code_craft`)
명확한 타입 시그니처와 불변식을 주고 Claude Sonnet 5.5 / Opus 5.5 Medium에 구현 위임:
```python
claude_code_craft(
    task_description="토큰 버킷 레이트 리미터 클래스를 구현하라.",
    code_context="<기존 인터페이스 및 관련 DTO 정의>",
    workspace_dir="/path/to/workspace",
    claude_model="claude-sonnet-5-5-medium",
)
```

### 워크플로우 D: 타임아웃/비동기 작업 조회 및 이어달리기
1. **타임아웃/비동기 잡 조회**:
   - `codex_job_status(job_id="cdx-...")` 또는 `codex_job_status(idempotency_key="...")`
   - 취소 시: `codex_job_status(job_id="cdx-...", cancel=True)`
2. **부분 성공(`partial`) 체크포인트 이어달리기**:
   - 일부 역할만 성공하고 타임아웃이 난 경우, `heterarchy_triad_orchestrate(..., resume_collaboration_id="cdx-...")`를 호출하면 이미 성공한 역할의 산출물은 재사용하고 미완료 역할만 재실행한다.

---

## 4. 라우팅·영수증·증거 범위 판독 계약

1. **검증된 라우팅 계약 (`validate_and_resolve_route`)**:
   - `codex_cli` 엔진에는 `CODEX_SUPPORTED_MODELS`(`gpt-6.1-sol`, `gpt-5.4`, `gpt-5-codex` 등)만 허용된다.
   - `antigravity_local_rpc` 엔진에서 Claude 모델은 `medium` 티어(`claude-sonnet-5-5-medium`, `claude-opus-5-5-medium`)만 허용하며, `high`/`low` 요청 시 `error_code="CLAUDE_TIER_POLICY_VIOLATION"`으로 거부한다.
2. **실행 영수증 (`receipt`)**:
   - 모든 응답은 `request_id`, `idempotency_key`, `provider`, `engine`, `requested_model`, `actual_model`, `model_mismatch`, `status`, `outcome_state`, `attempt_count`, `output_digest`(`sha256:...`), `result_ref`(`job://...`), `credentials_redacted=true`를 포함한다.
3. **증거 범위 고정 (`evidence_scope`)**:
   - 5대 검증 차원(`static_code_review`, `test_generation`, `test_execution`, `integration_verification`, `runtime_confirmation`)을 분리 기록한다.
   - `run_verification_gates=True`로 브리지가 직접 실행해 통과한 테스트만 `test_execution="verified"` (`test_execution_evidence_origin="bridge_executed"`)로 기록되며, 미실행 테스트는 항상 `"not_run"`으로 남긴다.
   - 워크스페이스에 `.codegraph/codegraph.db`가 존재할 때 `symbols_inspected`를 전달하면 해당 심볼의 정확한 코드 구간과 1-hop 호출 관계를 자동으로 추출해 프롬프트에 주입한다.

---

## 5. `hcodex` CLI 제어 명령어 요약

```bash
# 런타임, 플러그인, 스킬 등록 상태, 활성 PID 및 핸드오프 인박스 확인
uv run hcodex status

# 기존 stdio 프로세스 재기동 + 스킬/플러그인/MCP 설정 일괄 갱신
uv run hcodex restart

# 플러그인·스킬 및 MCP 설정 활성화 / 비활성화
uv run hcodex on
uv run hcodex off

# 3극 핸드오프 티켓 인박스(Argon / Sonnet / Opus / Codex) 조회
uv run hcodex inbox
```
