.DEFAULT_GOAL := ci
.PHONY: ci check harness ruff lint pyrefly typecheck test pytest tui status restart help

UV ?= uv

help: ## 사용 가능한 Make 명령어 목록 출력
	@printf "\033[1;36m=== Heterarchy Codex (codex-collab-mcp) Makefile Targets ===\033[0m\n"
	@printf "  \033[1;32mmake ci\033[0m         하네스(9개 기계적 계약) + ruff + pyrefly + pytest 통합 점검\n"
	@printf "  \033[1;32mmake check\033[0m      하네스 + ruff + pyrefly 정적 점검 (빠른 검사)\n"
	@printf "  \033[1;32mmake harness\033[0m    .agents/python_dev_harness 9개 기계적 계약 검증\n"
	@printf "  \033[1;32mmake ruff\033[0m       Ruff 린트 점검 (app, tests)\n"
	@printf "  \033[1;32mmake pyrefly\033[0m    Pyrefly 엄격 정적 타입 점검\n"
	@printf "  \033[1;32mmake test\033[0m       Pytest 전체 테스트 스위트 실행\n"
	@printf "  \033[1;32mmake tui\033[0m        hcodex tui 대화형 워크벤치 실행\n"
	@printf "  \033[1;32mmake status\033[0m     hcodex status 브릿지 및 모델 상태 확인\n"
	@printf "  \033[1;32mmake restart\033[0m    hcodex restart 데몬 재시작\n"

harness: ## 9개 하네스 기계적 계약 검증 (.agents/python_dev_harness)
	@printf "\033[1;36m[1/4] Running Alexandria Python Dev Harness (9 Mechanical Contracts)...\033[0m\n"
	$(UV) run python .agents/python_dev_harness/verify_mechanical_contracts.py

ruff: ## Ruff 린트 점검
	@printf "\033[1;36m[2/4] Running Ruff Lint Check (app, tests)...\033[0m\n"
	$(UV) run ruff check app tests

lint: ruff

pyrefly: ## Pyrefly 정적 타입 점검
	@printf "\033[1;36m[3/4] Running Pyrefly Strict Type Check...\033[0m\n"
	$(UV) run pyrefly check

typecheck: pyrefly

test: ## Pytest 테스트 스위트 실행
	@printf "\033[1;36m[4/4] Running Pytest Suite...\033[0m\n"
	$(UV) run pytest -q

pytest: test

check: harness ruff pyrefly ## 빠른 정적 검증 (하네스 + ruff + pyrefly)
	@printf "\033[1;32m✔ [CHECK PASS] Harness + Ruff + Pyrefly checks passed!\033[0m\n"

ci: harness ruff pyrefly test ## 전체 CI 파이프라인 (하네스 + ruff + pyrefly + pytest)
	@printf "\033[1;32m✔ [CI PASS] All 9 Harness Contracts + Ruff + Pyrefly + Pytest gates passed!\033[0m\n"

tui: ## hcodex TUI 워크벤치 실행
	$(UV) run hcodex tui

status: ## hcodex 상태 점검
	$(UV) run hcodex status

restart: ## hcodex MCP 데몬 재시작
	$(UV) run hcodex restart
