# Contributing to Agent Browser

Thank you for considering contributing to Agent Browser!

## How to Contribute

### Reporting Bugs

If you find a bug, please create an issue on [GitHub Issues](https://github.com/EchoYue/agent-browser/issues) with:

1. A clear title and description
2. Steps to reproduce
3. Expected behavior vs actual behavior
4. Environment info (OS, Rust version, Chrome version)
5. Relevant logs or error messages

### Suggesting Features

Feature suggestions are welcome! Please describe in an issue:

1. Feature description
2. Use cases
3. Possible implementation approach

### Submitting Code

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Create a Pull Request

### Code Guidelines

- Use `cargo fmt` to format code
- Use `cargo clippy` to check code quality
- Add necessary comments and documentation
- Add tests for new features

### Commit Message Format

Use clear commit messages:

- `feat: add new feature`
- `fix: fix bug`
- `docs: update documentation`
- `refactor: refactor code`
- `test: add tests`
- `chore: build/tooling related`

### Development Setup

```bash
# Clone the repository
git clone https://github.com/EchoYue/agent-browser.git
cd agent-browser

# Install dependencies and build
cargo build

# Run tests
cargo test

# Run clippy
cargo clippy

# Format code
cargo fmt
```

## License

By contributing code, you agree that your code will be licensed under the MIT License.