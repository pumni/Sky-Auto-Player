//! Skeleton only. Implement according to the migration guide.
#![deny(unsafe_op_in_unsafe_fn)]

use pyo3::prelude::*;
use std::sync::Arc;

#[pyclass(frozen)]
struct DispatchSession {
    inner: Arc<SessionShared>,
}

struct SessionShared {
    // lifecycle/channel/atomics/snapshots; no PyObject fields.
}

#[pymethods]
impl DispatchSession {
    #[staticmethod]
    fn prepare(/* Bound Python DTO inputs */) -> PyResult<Self> {
        todo!("validate/extract to native data, then build native session")
    }

    fn start(&self) -> PyResult<()> {
        todo!("spawn dedicated Rust worker")
    }

    fn send_command(&self, command: &str) -> PyResult<bool> {
        let _ = command;
        todo!("strict enum parse + bounded try_send + signal event")
    }

    fn join(&self, py: Python<'_>, timeout_ms: Option<u64>) -> PyResult<Py<PyAny>> {
        let _native_result = py.detach(|| {
            let _ = timeout_ms;
            todo!("join without accessing Python")
        });
        todo!("convert native result after the thread is attached again")
    }
}

#[pyfunction]
fn build_info() -> (&'static str, &'static str) {
    (env!("CARGO_PKG_VERSION"), "schema-v1")
}

#[pymodule(gil_used = false)]
fn sky_player_rs(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DispatchSession>()?;
    module.add_function(wrap_pyfunction!(build_info, module)?)?;
    Ok(())
}
