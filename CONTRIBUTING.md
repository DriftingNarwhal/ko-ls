# Contributing

## Licensing of contributions

This project is **licensed under the GNU Affero General Public License v3.0 (`LICENSE`)**. By contributing you agree your contribution is licensed
under those same terms.


## The gate

`cargo test --workspace` and `cargo clippy --workspace --all-targets` must both be clean
before a change lands. A run that skipped clippy because the toolchain lacked it has
checked half the gate and should say so.
