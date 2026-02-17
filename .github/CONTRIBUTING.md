# Contributing to SmartopolAI

Thank you for your interest in contributing! SmartopolAI is an open-source project and we welcome contributions from the community.

## Language

All code, comments, documentation, commit messages, and PR descriptions **must be in English**.

## Development Setup

### Prerequisites

- Rust 1.80+ (install via [rustup](https://rustup.rs/))
- Git

### Build

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai/skynet
cargo check    # fast type checking
cargo test     # run all tests
cargo clippy   # lint
```

### Development Workflow

```bash
cargo check          # after every change (fast, no binary)
cargo test           # before committing
cargo clippy         # before committing
cargo run            # to test locally
```

Do **not** use `cargo build --release` during development — it's slow and unnecessary.

## Commit Convention

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add WebSocket handshake state machine
fix: correct payload size overflow check
docs: add API reference for /ws endpoint
refactor: extract auth verification into module
test: add wire protocol compatibility tests
chore: update dependencies
```

**Rules:**
- Lowercase type prefix (`feat:`, not `Feat:`)
- Imperative mood in subject (`add feature`, not `added feature`)
- No period at end of subject line
- Body (optional) explains **why**, not what

## Branch Strategy

```
main            ← stable, protected, releases tagged here
└── develop     ← integration branch
     ├── feat/description
     ├── fix/description
     └── docs/description
```

1. Create a branch from `develop`: `git checkout -b feat/my-feature develop`
2. Make your changes with conventional commits
3. Open a PR against `develop`
4. After review and CI pass, it gets merged

## Pull Request Process

1. Ensure `cargo check`, `cargo test`, and `cargo clippy -- -D warnings` all pass
2. Update relevant documentation if behavior changes
3. Add tests for new functionality
4. Keep PRs focused — one feature or fix per PR
5. Fill out the PR template

## Code Style

- Follow standard Rust formatting (`rustfmt`)
- Comments explain **why**, not what
- No premature abstraction — explicit and readable over clever
- English only in all code artifacts

## Reporting Issues

Use GitHub Issues with the provided templates:
- **Bug Report** — for unexpected behavior
- **Feature Request** — for new functionality proposals

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
