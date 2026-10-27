# `codex-collab-mcp` (CASOAS)

[![github action](https://github.com/limhaneul12/casoas/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/limhaneul12/casoas/actions/workflows/ci.yml)
![Python 3.14t](https://img.shields.io/badge/python-3.14t-blue)
![Pydantic v2.14.1](https://img.shields.io/badge/pydantic-v2.14.1-e92063)
![SQLite WAL](https://img.shields.io/badge/sqlite-WAL-003B57)
![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)
![Pyrefly](https://img.shields.io/badge/type%20checker-pyrefly%20strict-purple)

**Gemini 4 Argon (Antigravity) ↔ Claude Sonnet/Opus 5.5 ↔ OpenAI Codex (`codex-cli`)** 3극(Tri-Polar) 협업을 위한 프로덕션 MCP 브릿지 서버입니다.  
단일 모델이 설계·구현·감사를 혼자 수행할 때 발생하는 확증 편향을 제거하고, **설계(Argon) → 정밀 구현(Sonnet/Opus) → 적대적 감사(Codex) → 합의 검증** 파이프라인을 자동화합니다.

> **📖 에이전트 스킬 및 상세 사용 매뉴얼**: [`skills/codex-heterarchy/SKILL.md`](./skills/codex-heterarchy/SKILL.md)

---

## 1. 왜 필요한가? (Overview)

복잡한 코드베이스를 리팩토링하거나 핵심 비즈니스 로직을 작성할 때, 각 AI 모델의 강점을 역할별로 분리하여 교차 검증합니다.

| 노드 (Pole) | 담당 역할 | 핵심 강점 |
| :--- | :--- | :--- |
| **Gemini 4 Argon** | **Macro-Architect & Orchestrator** | 저장소 전체 컨텍스트 파악, 아키텍처 불변식 수립, 최종 합의 판정 |
| **Claude Sonnet 5.5 / Opus 5.5 (`medium`)** | **Surgical Code Craftsman** | 정교한 다중 파일 리팩토링, 클린 모듈 구현, API/UI 설계 (`claude_code_craft`) |
| **OpenAI Codex (`codex-cli`)** | **Adversarial Red-Team Auditor** | 엣지 케이스 파괴, 동시성·보안 결함 탐지, 핀포인트 코드/테스트 합성 |

---

## 2. 핵심 기술 스택 및 특징 (Architecture & Stack)

- **Python `3.14t` Free-Threading (No-GIL)**
  - GIL이 비활성화된 Python `3.14t` 런타임에서 `ThreadPoolExecutor`와 `threading.RLock()`을 활용해 다중 파일 SHA-256 증거 해싱과 CodeGraph 심볼 슬라이싱을 병렬 처리합니다.
- **Pydantic `v2.14.1` (`>=2.7.0`) 엄격 스키마 검증**
  - 모든 외부 입출력 경계에 `StrictSchemaModel`(`extra="forbid"`, `frozen=True`, `validate_default=True`)을 적용하여 오염된 페이로드나 허용되지 않은 필드를 사전 차단합니다.
- **SQLite (WAL 모드) 자동 영속화**
  - MCP 서버 시작 시 `~/.cache/codex-collab-mcp/` 하위에 `jobs.db`와 `handoffs.db`(`WAL` 모드)를 자동 생성·초기화합니다.
  - 실행 영수증(Receipt), 멱등성 키(`idempotency_key`), 타임아웃 복구 상태(`timed_out_unknown`), 체크포인트 이어달리기(`resume_collaboration_id`)를 안전하게 보존합니다.

### 기능별 패키지 구조 (`src/codex_collab_mcp/`)

```text
src/codex_collab_mcp/
├── routing/         # 모델 스펙/별칭, 사전 라우팅 검증, 실행 영수증(Receipt), 예산(Budget), 시크릿 마스킹
├── storage/         # SQLite(WAL) 작업(Job) 및 핸드오프(Handoff) 엔진 & 불변 레코드 DTO
├── evidence/        # 병렬 SHA-256 파일 해싱, CodeGraph 심볼 슬라이싱, 증거 범위 및 3극 합의 판정
├── transports/      # Antigravity 로컬 RPC, Codex CLI 서브프로세스/JSONL 파서, Git 리뷰, HTTP 폴백
├── orchestration/   # MCP 실행 게이트웨이, 3극 Pipeline/Council 오케스트레이터, 검증 게이트, Alexandria 패키징
├── server.py        # FastMCP 도구 엔트리포인트 (15 Tools)
├── schemas.py       # Pydantic v2 StrictSchemaModel 입출력 경계 스키마
└── ctl.py           # `hcodex` 원클릭 CLI 컨트롤러
```

---

## 3. 빠른 시작 (Quick Start)

### 사전 요구사항
- **Python**: `3.14t` (Free-Threading) 및 [`uv`](https://docs.astral.sh/uv/)
- **Pydantic**: `v2.14.1` (`pydantic>=2.7.0`, `uv sync` 시 자동 설치)
- **OpenAI Codex CLI**: `codex` 바이너리 설치 및 로그인 (`codex login`)

### 설치 및 원클릭 등록

```bash
# 1. 저장소 클론 및 의존성 설치
git clone https://github.com/limhaneul12/casoas.git
cd casoas
uv sync

# 2. Antigravity & Codex 글로벌 스킬 및 MCP 서버 자동 등록
uv run hcodex on

# 3. 런타임, 플러그인, 스킬, 활성 PID 및 핸드오프 상태 점검
uv run hcodex status
```

`uv run hcodex on` 명령 한 번으로 현재 클론된 경로에 맞춰 아래 설정이 자동 등록됩니다:
- **Antigravity 스킬**: `~/.gemini/config/skills/codex-heterarchy/SKILL.md`
- **Codex 스킬**: `~/.codex/skills/codex-heterarchy/SKILL.md`
- **Antigravity MCP 설정**: `~/.gemini/config/plugins/heterarchy-codex/mcp_config.json`
- **Codex MCP 설정**: `~/.codex/config.toml` (`[mcp_servers.heterarchy_codex]`)

---

## 4. 제공 MCP 도구 (15 Tools)

### 🏛️ 3극 오케스트레이션 & 의사결정
| 도구명 | 설명 |
| :--- | :--- |
| `heterarchy_triad_orchestrate` | `pipeline`(순차 구현·감사·판정) 또는 `council`(병렬 3자 리뷰) 오케스트레이션. 체크포인트 이어달리기(`resume_collaboration_id`), 자동 수리 루프(`max_rounds=1..3`), 검증 게이트(`run_verification_gates=True`) 지원 |
| `codex_alexandria_decision_pack` | 3극 합의 결과(`collaboration_id`)와 증거 스코프(`expected_scope_digest`)를 검증하여 Alexandria 영구 메모리 업서트용 의사결정 패키지로 변환 |

### 🛠️ 정밀 코드 구현 & 합성
| 도구명 | 설명 |
| :--- | :--- |
| `claude_code_craft` | Claude Sonnet 5.5 (`claude-sonnet-5-5-medium`) 또는 Opus 5.5 (`claude-opus-5-5-medium`)를 호출하여 정밀 코드 구현 및 리팩토링 수행 |
| `codex_targeted_synthesize` | 명확한 시그니처와 계약 조건을 바탕으로 군더더기 없는 프로덕션 코드와 경계값 단위 테스트 합성 |
| `codex_differential_implementations` | 성능(`latency_and_throughput`), 메모리(`memory_footprint`), 유지보수성(`maintainability_and_clarity`) 축별로 대안 구현체를 동시 생성 및 비교 |

### 🛡️ 적대적 감사 & 디버깅
| 도구명 | 설명 |
| :--- | :--- |
| `codex_adversarial_audit` | 코드/Diff를 레드팀 관점에서 감사하여 `verdict`(`PASS`/`WARN`/`BLOCK`), 위험도 점수, 취약점 및 재현 테스트 케이스 반환 |
| `codex_rescue_sandbox` | 해결되지 않는 버그나 테스트 실패에 대해 격리된 원인 분석 및 최소 패치 도출 (`async_background=True` 지원) |
| `codex_git_review` | 로컬 Git 변경사항(`uncommitted`, `base_branch`, `commit_sha`)에 대한 네이티브 Codex 코드 리뷰 실행 |

### 🔄 세션 · 작업 · 핸드오프 관리
| 도구명 | 설명 |
| :--- | :--- |
| `codex_session_followup` | 기존 Codex 세션(`session_id`)을 이어서 실행(`resume`)하거나 새 갈래로 분기(`fork`) |
| `codex_job_status` / `codex_job_list` | SQLite에 기록된 비동기/타임아웃(`timed_out_unknown`) 작업 조회, 멱등성 확인 및 안전한 취소(`cancel=True`) |
| `heterarchy_handoff_create` / `inbox` / `resolve` | `argon`, `sonnet`, `opus`, `codex` 간 비동기 작업 위임 및 해결 상태를 공유하는 SQLite 블랙보드 티켓 시스템 |
| `codex_bridge_health` | 로컬 `codex` CLI, Antigravity RPC 라우팅 계약(`routable_*`), 인증 격리 및 SQLite 저장소 상태 진단 |

---

## 5. 핵심 검증 원칙 및 품질 게이트 (Verification & Quality Gates)

1. **수행 상태와 합의 상태의 엄격한 분리**:
   - 모든 역할이 명시적 `verdict`(`PASS` / `WARN` / `BLOCK`)와 요약 본문을 반환해야만 정적 리뷰가 `verified`로 인정되며, 전원 `PASS`일 때만 `review_approved=True`가 부여됩니다.
2. **증거 범위(Evidence Scope) 고정**:
   - 파일 SHA-256 해시(`files_inspected_digests`), CodeGraph 슬라이싱 컨텍스트, 브릿지가 직접 실행한 검증 게이트 결과만 신뢰하며, 실행하지 않은 항목은 `"not_run"`으로 정직하게 유지합니다.
3. **안전한 프로세스 제어 (`hcodex`)**:
   - 코드 업데이트 후 `uv run hcodex restart`는 설정과 스킬을 동기화하며, 실행 중인 다른 에이전트 세션을 강제 종료하지 않습니다.

### 로컬 품질 검증 명령어

```bash
# 코드 포맷 및 린트 검사 (Ruff)
uv run ruff format --check src tests
uv run ruff check src tests

# 정적 타입 체크 (Pyrefly Strict)
uv run pyrefly check

# 전체 테스트 스위트 실행 (Pytest, 73 tests)
uv run pytest -v
```
