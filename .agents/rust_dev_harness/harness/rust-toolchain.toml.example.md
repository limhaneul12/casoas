# rust-toolchain.toml.example

Repository-owned Rust toolchain policy 예시다.

```toml
[toolchain]
channel = "<PINNED_STABLE_OR_REVIEWED_CHANNEL>"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

## Policy

- production/CI 재현성이 중요하면 reviewed stable version을 pin한다.
- rolling stable을 선택하더라도 그 정책을 `PROJECT_PROFILE.md`에 명시하고 CI/toolchain update가 우연한 workstation 상태에 의해 결정되지 않게 한다.
- nightly는 Miri, fuzz, compiler feature 등 명확한 이유가 있는 별도 extended lane에서 사용한다. ordinary production build authority를 이유 없이 nightly로 바꾸지 않는다.
- `rust-version`/MSRV와 toolchain pin은 다른 계약이다. MSRV는 지원 가능한 최소 compiler이고, toolchain pin은 해당 repository가 현재 검증하는 compiler다.

## Effective-toolchain contract

`rust-toolchain.toml`을 두는 것만으로 충분하다고 가정하지 않는다. `rustup` proxy가 아닌 Homebrew/system `cargo` 또는 `rustc`가 PATH에서 먼저 선택되면 repository pin이 우회될 수 있다.

공용 xtask는 toolchain file이 존재할 때 다음을 fail-closed로 검증한다.

```text
rust-toolchain.toml [toolchain].channel
        ↓
rustup run <channel> rustc/cargo --version
        ↕ exact resolution check
ambient PATH rustc/cargo --version
```

- declared channel이 설치되어 있지 않거나 `rustup`으로 resolve되지 않으면 PASS하지 않는다.
- declared와 ambient tool identity가 다르면 `cargo xtask rules`와 `doctor`가 mismatch를 보고한다.
- build/check/test/metadata/deps/docs/bench 같은 harness child command는 raw PATH cargo가 아니라 declared channel runner를 공통 사용한다.
- toolchain file이 없는 repository는 `PROJECT_PROFILE.md`에 rolling/current-stable 정책과 그 이유를 명시한다.
- toolchain mismatch를 자동으로 숨기거나 다른 compiler로 fallback하지 않는다.
