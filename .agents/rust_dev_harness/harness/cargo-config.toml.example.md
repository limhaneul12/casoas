# cargo-config.toml.example

Repository root의 `.cargo/config.toml`에 적용하는 최소 `xtask` alias 예시다.

```toml
[alias]
xtask = "run --package xtask --"
```

## Rule

- canonical mechanical entrypoint는 `cargo xtask <command>`다.
- shell script나 Makefile이 이 alias를 재구현하지 않는다.
- polyglot 상위 task runner가 필요하면 이 명령을 호출하는 얇은 adapter만 둔다.
- `rust-toolchain.toml`이 있는 repository에서는 `cargo`/`rustc`가 rustup proxy를 통해 declared channel과 동일하게 resolve되어야 한다.
- Homebrew/system Cargo가 PATH에서 repository pin을 우회하면 `cargo xtask rules`가 effective-toolchain mismatch로 fail해야 한다.
- xtask 내부의 Cargo child process는 declared toolchain runner를 사용하므로 `metadata`, `fmt`, `check`, `clippy`, `test`, `deps`, docs, bench가 서로 다른 compiler를 사용하지 않는다.

## Why the alias remains simple

`.cargo/config.toml`에 특정 버전 문자열을 중복 hard-code하지 않는다. toolchain authority는 `rust-toolchain.toml` 한 곳에 두고 xtask가 그 값을 읽어 실행을 라우팅한다. 이렇게 해야 toolchain update 시 alias, CI, Makefile, script 사이의 version drift를 만들지 않는다.
