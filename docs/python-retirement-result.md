# Python 완전 제거 결과

2026-11-03 · 프로젝트 범위: `intelligent-oppenheimer`

## 직접 사용 확인

이 채팅에 연결된 **Rust MCP**의 health, job readback, DAG plan-only를 직접 호출했습니다. DAG는 8개 노드·3개 wave·blocking hazard 0으로 정상 계획됐습니다. 설치된 Rust CLI에서도 models, status, permissions, jobs를 사용했습니다.

Argon High·Claude Sonnet 5.5 Medium·GPT-6.1 Sol의 실제 read-only council 검토가 모두 성공했습니다. 판정은 PASS/WARN/WARN이며, 합의나 승인으로 부풀리지 않았습니다. 두 WARN은 제한된 source excerpt와 미실행 filesystem 점검에 관한 것이었습니다. 별도 로컬 inventory, 독립 삭제 계획 검토 PASS, 실제 삭제 전 검사와 삭제 후 CI로 그 조건을 확인했습니다.

## 제거 범위

- 프로젝트 `.venv/`
- `.pytest_cache/`, `.ruff_cache/`
- 이전 Python 구현·테스트·패키징 백업 전체
- 이전 Python 개발 하네스 백업 전체
- 과거 `.omx/evidence/` Python 검증 클라이언트와 bytecode
- Python 전용 `.gitignore` 항목과 더 이상 유효하지 않은 oracle 문구

**5개 디렉터리와 Python helper 12개를 포함해 2,404개 파일, logical 96,985,646 bytes를 제거했습니다.**

프로젝트 전체를 다시 열거했을 때 Git 이력을 제외한 `.py`, `.pyc`, `.pyo`, `.pyi` 파일은 **0개**입니다. `app/`, root Python `tests/`, `pyproject.toml`, `uv.lock`, `.python-version` 역시 없습니다.

## 보존한 항목

- 실제 job/handoff 데이터와 DB·sidecar
- Rust에서 사용하는 과거 데이터 호환 SQLite fixture
- JSON·로그 영수증, source hash/provenance metadata
- 보호된 설치 설정 백업과 관련 없는 기존 작업
- 프로젝트 밖의 공유 Python 설치·환경

삭제 직후뿐 아니라 작업 종료 시에도 보존 대상으로 고정한 **158개 파일의 SHA-256이 모두 동일**함을 확인했습니다. 삭제한 이전 source/harness archive의 JSON provenance는 새 evidence 경로로 옮겨 source hash 기록만 남겼습니다. Python 코드 자체의 백업은 남기지 않았습니다.

외부 Python workspace를 대상으로 분석·검증하는 Rust 기능과 역사 데이터 reader는 제품 계약이므로 유지했습니다. 이는 이 프로젝트의 Python 실행 의존성이 아닙니다.

## 관련 단순화

스킬의 oracle 문구를 제거하는 과정에서 embedded asset 해시가 Rust 상수와 `PROVENANCE.json`에 중복 선언된 문제가 CI에 잡혔습니다. 중복 상수를 삭제하고 compiled provenance manifest를 유일한 예상 해시 authority로 정리했습니다.

실제 embedded bytes의 SHA-256 검증, 정확한 inventory, repository/embedded byte equality는 유지했습니다. 변조된 source asset이 거부되는 회귀 테스트와 독립 무결성 검토 PASS를 확인했습니다. 새로운 의존성이나 runtime abstraction은 추가하지 않았습니다.

## 삭제 후 검증

- `make ci`: **149/149 통과** — 145 library + 2 실제 binary stdio integration + 2 xtask.
- Rust harness/manifest, fmt, workspace check, strict Clippy `-D warnings` 통과.
- 설치된 CLI models/status/jobs 재사용 정상.
- 연결된 MCP health/job readback 정상; Codex CLI 0.166.0과 Antigravity routable 상태 확인.
- 새 Rust 바이너리 재설치 및 Rust MCP 설정 확인.
- canonical/plugin/embedded/global Codex·Gemini skill 내용과 SHA-256 일치.
- 활성 MCP command는 `/Users/imhaneul/.cargo/bin/hcodex`, args는 `["mcp"]`; 비-Python `CODEX_CLI_BIN` 보존.

실행·삭제 명세와 결과: `.omx/evidence/python-retirement-20261103/`의 `deletion-manifest.json`, `deletion-result.json`, `verification.json`, `ci-after.log`, `install.log`, `health-post-delete.json`, `job-post-delete.json` 및 CLI/read-only role 기록.

첫 CI의 asset digest 오류는 수정 전 진단으로 `ci.log`에 보존했습니다. 성공 결과는 `ci-after.log`입니다. 기존 runtime/provider evidence의 Python helper 재실행 경로는 폐기됐지만, 실제 JSON·로그 영수증은 그대로 보존됐습니다.
