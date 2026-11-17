use crate::model::RunResult;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RunTicket {
    revision: u64,
    serial: u64,
}

#[derive(Default)]
struct ActiveRun {
    revision: u64,
    serial: u64,
    result: Option<RunResult>,
}

/// Only results published by the current desktop calculation can be queried.
/// CLI and persisted historical manifests keep their existing format.
#[derive(Default, Clone)]
pub(crate) struct ResultRegistry(Arc<Mutex<ActiveRun>>);

impl ResultRegistry {
    pub(crate) fn with_current<T>(
        &self,
        batch: &str,
        publish: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self.0.lock().map_err(|_| "无法锁定当前计算状态")?;
        if state
            .result
            .as_ref()
            .is_none_or(|result| result.run_dir != batch)
        {
            return Err("处理结果已失效，请重新计算和预检导出范围".into());
        }
        publish()
    }
    pub(crate) fn reset(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.revision = 0;
            state.serial += 1;
            state.result = None;
        }
    }
    pub(crate) fn invalidate(&self, revision: u64) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "无法锁定当前计算状态")?;
        if revision > state.revision {
            state.revision = revision;
            state.result = None;
        }
        Ok(())
    }

    pub(crate) fn begin(&self, revision: u64) -> Result<RunTicket, String> {
        let mut state = self.0.lock().map_err(|_| "无法锁定当前计算状态")?;
        if revision < state.revision {
            return Err("处理规则已变化，请重新计算".into());
        }
        state.revision = revision;
        state.serial += 1;
        state.result = None;
        Ok(RunTicket {
            revision,
            serial: state.serial,
        })
    }

    pub(crate) fn publish(
        &self,
        ticket: RunTicket,
        result: RunResult,
    ) -> Result<RunResult, String> {
        let mut state = self.0.lock().map_err(|_| "无法锁定当前计算状态")?;
        if ticket.revision != state.revision || ticket.serial != state.serial {
            return Err("本次计算已经失效，请重新运行".into());
        }
        state.result = Some(result.clone());
        Ok(result)
    }

    pub(crate) fn resolve(&self, batch: &str) -> Result<RunResult, String> {
        let state = self.0.lock().map_err(|_| "无法锁定当前计算状态")?;
        state
            .result
            .as_ref()
            .filter(|result| result.run_dir == batch)
            .cloned()
            .ok_or_else(|| "处理结果已失效，请重新计算".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: &str) -> RunResult {
        RunResult {
            run_dir: id.into(),
            database: format!("{id}/result.duckdb"),
            reviews: vec![],
            input_fingerprints: Default::default(),
        }
    }

    #[test]
    fn older_calculation_cannot_replace_new_or_invalidated_result() {
        let registry = ResultRegistry::default();
        let first = registry.begin(1).unwrap();
        let next = registry.begin(1).unwrap();
        assert!(registry.publish(first, result("old")).is_err());
        registry.publish(next, result("new")).unwrap();
        assert_eq!(
            registry.resolve("new").unwrap().database,
            "new/result.duckdb"
        );
        assert!(registry.resolve("old").is_err());
        registry.invalidate(2).unwrap();
        assert!(registry.resolve("new").is_err());
        assert!(registry.publish(next, result("late")).is_err());
        assert!(registry.begin(1).is_err());
        registry.reset();
        assert!(registry.begin(0).is_ok());
        assert!(registry.publish(next, result("late_after_reload")).is_err());
    }
}
