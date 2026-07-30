# 09 — Build, wheel, uv và PyInstaller

## 1. Giữ root Python build ổn định

Root `pyproject.toml` hiện dùng setuptools. Không bắt buộc đổi toàn dự án sang maturin. Khuyến nghị build native wheel riêng trong `rust/`, install vào cùng uv environment trước test/freeze.

```text
root setuptools package: sky-auto-player
native wheel: sky-player-rs
extension module: sky_player_rs.pyd
```

## 2. rust/pyproject.toml

```toml
[build-system]
requires = ["maturin>=1.13.3,<2"]
build-backend = "maturin"

[project]
name = "sky-player-rs"
version = "0.1.0"
requires-python = ">=3.14,<3.15"

[tool.maturin]
manifest-path = "Cargo.toml"
bindings = "pyo3"
module-name = "sky_player_rs"
strip = true
```

Không thêm `compatibility = abi3`, không enable PyO3 abi3.

## 3. Toolchain

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
targets = ["x86_64-pc-windows-msvc"]
components = ["rustfmt", "clippy"]
```

Build release dùng MSVC toolchain để tương thích Windows Python.

## 4. Build script

Tạo `scripts/build_rust_wheel.py`:

- verify `sys.version_info == (3,14,...)`;
- verify free-threaded build/runtime flags;
- run `maturin build --release --interpreter sys.executable`;
- locate exactly one wheel;
- verify filename tag có `cp314` và free-threaded ABI tag;
- `uv pip install --reinstall <wheel>`;
- import `sky_player_rs` và check `build_info()`;
- fail hard nếu module làm GIL bật lại.

Không shell out qua bare `python`; dùng `sys.executable`/`uv run` để không build nhầm stock interpreter.

## 5. Development

```powershell
uv run maturin develop --manifest-path rust/Cargo.toml --release
uv run python -c "import sky_player_rs; print(sky_player_rs.build_info())"
uv run pytest -q
```

`maturin develop` chỉ dev. Release pipeline build wheel và install wheel để test artifact thật.

## 6. PyInstaller

Trước freeze:

1. install native wheel vào build environment;
2. import extension qua static import trong `native_dispatch.py` để PyInstaller discover;
3. nếu analysis không collect `.pyd`, thêm explicit hidden import/binary collection;
4. selftest frozen app import module và chạy dry-run session;
5. không copy random `.pyd` từ target path bằng wildcard không versioned.

Spec phải giữ extension cạnh app theo PyInstaller layout. Wheel metadata không cần ship, nhưng `.pyd` và dependent VC runtime phải resolve.

## 7. Version reporting

Doctor/telemetry thêm:

```text
rust_core_enabled
rust_core_version
rustc_version
pyo3_version
native_abi
native_build_commit
native_schema_version
```

`native_build_commit` lấy build-time env từ Git SHA. Mismatch Python commit/schema phải fail prepare thay vì chạy undefined behavior.

## 8. CI jobs

### Core cross-platform

- stable pinned 1.97.1;
- fmt/check/clippy/test pure core;
- no Python needed cho `sky_dispatch_core`.

### Windows native

- Windows x64;
- project-pinned CPython 3.14t;
- build/install wheel;
- import/no-GIL smoke;
- pytest;
- Windows integration marker;
- PyInstaller selftest.

### Artifact

- wheel filename/tag validation;
- SBOM/dependency audit;
- frozen app smoke;
- hash artifacts.

## 9. Dependency policy

Pin critical ABI dependencies exact (`pyo3`, `windows-sys`) trong migration. Các utility crates có semver range nhưng `Cargo.lock` committed. Renovate/Dependabot update qua PR có full native test.

Không dùng git dependencies trên release branch trừ emergency pinned commit và documented removal plan.
