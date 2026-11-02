//! Windows GPU Engine counters, grouped by adapter LUID and hardware engine.
//! Process usage is summed within one engine, then the busiest engine is shown.
use super::GpuStats;
use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::System::Performance::*;

pub struct GpuCounters {
    query: isize,
    engines: isize,
    memory: isize,
}
impl GpuCounters {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut query = 0;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut value = Self {
                query,
                engines: 0,
                memory: 0,
            };
            let engine_path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
                .encode_utf16()
                .collect();
            if PdhAddEnglishCounterW(query, PCWSTR(engine_path.as_ptr()), 0, &mut value.engines)
                != 0
            {
                return None;
            }
            let memory_path: Vec<u16> = "\\GPU Adapter Memory(*)\\Dedicated Usage\0"
                .encode_utf16()
                .collect();
            let _ =
                PdhAddEnglishCounterW(query, PCWSTR(memory_path.as_ptr()), 0, &mut value.memory);
            let _ = PdhCollectQueryData(query);
            Some(value)
        }
    }
    pub fn sample(&mut self, gpus: &mut [GpuStats]) {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return;
        }
        let engines = self.values(self.engines);
        let memory = self.values(self.memory);
        for gpu in gpus {
            gpu.usage_percent = aggregate_engine_usage(&gpu.adapter_id, &engines);
            gpu.memory_used_bytes = adapter_memory_usage(&gpu.adapter_id, &memory);
        }
    }
    fn values(&self, counter: isize) -> Vec<(String, f64)> {
        if counter == 0 {
            return vec![];
        }
        unsafe {
            let mut bytes = 0;
            let mut count = 0;
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, None)
                != PDH_MORE_DATA
                || bytes == 0
                || bytes > 8 * 1024 * 1024
            {
                return vec![];
            }
            // u64 storage ensures the native records have their required alignment.
            let mut storage = vec![0u64; (bytes as usize).div_ceil(8)];
            let ptr = storage.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            let capacity = bytes as usize;
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                Some(ptr),
            ) != 0
                || count as usize > capacity / std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
            {
                return vec![];
            }
            let base = storage.as_ptr() as usize;
            let end = base + capacity;
            std::slice::from_raw_parts(ptr, count as usize)
                .iter()
                .filter_map(|item| {
                    if !matches!(
                        item.FmtValue.CStatus,
                        PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                    ) {
                        return None;
                    }
                    let address = item.szName.0 as usize;
                    if address < base || address >= end || !address.is_multiple_of(2) {
                        return None;
                    }
                    let words = std::slice::from_raw_parts(item.szName.0, (end - address) / 2);
                    let length = words.iter().position(|word| *word == 0)?;
                    let name = String::from_utf16_lossy(&words[..length]).to_lowercase();
                    Some((name, item.FmtValue.Anonymous.doubleValue))
                })
                .collect()
        }
    }
}

fn adapter_memory_usage(adapter: &str, samples: &[(String, f64)]) -> Option<u64> {
    // GPU Adapter Memory contains adapter totals, unlike GPU Process Memory.
    // Sum distinct physical nodes for a linked adapter, never process/shared mappings.
    let prefix = format!("{adapter}_phys_");
    let mut nodes = HashMap::new();
    for (name, value) in samples {
        let Some(node) = name.strip_prefix(&prefix) else {
            continue;
        };
        if node.is_empty()
            || !node.bytes().all(|byte| byte.is_ascii_digit())
            || !value.is_finite()
            || *value < 0.0
        {
            continue;
        }
        nodes.entry(node).or_insert(*value as u64);
    }
    if nodes.is_empty() {
        None
    } else {
        Some(
            nodes
                .values()
                .fold(0u64, |total, value| total.saturating_add(*value)),
        )
    }
}
impl Drop for GpuCounters {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

fn aggregate_engine_usage(adapter: &str, samples: &[(String, f64)]) -> Option<f32> {
    let mut engines = HashMap::<String, f64>::new();
    for (name, value) in samples {
        if adapter.is_empty() || !name.contains(adapter) || !value.is_finite() || *value < 0.0 {
            continue;
        }
        let Some((_, suffix)) = name.split_once("_eng_") else {
            continue;
        };
        let engine = suffix.split('_').next().unwrap_or("");
        if engine.is_empty() {
            continue;
        }
        *engines.entry(engine.to_owned()).or_default() += value;
    }
    engines
        .values()
        .copied()
        .reduce(f64::max)
        .map(|v| v.clamp(0.0, 100.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_uses_unique_adapter_nodes_and_excludes_process_instances() {
        let samples = vec![
            ("luid_a_phys_0".into(), 100.0),
            ("luid_a_phys_1".into(), 50.0),
            ("luid_a_phys_0".into(), 100.0),
            ("pid_1_luid_a_phys_0".into(), 80.0),
            ("pid_2_luid_a_phys_0".into(), 80.0),
            ("luid_b_phys_0".into(), 900.0),
        ];
        assert_eq!(adapter_memory_usage("luid_a", &samples), Some(150));
        assert_eq!(adapter_memory_usage("missing", &samples), None);
    }
    #[test]
    fn aggregates_processes_per_engine_then_chooses_busiest() {
        let values = vec![
            ("pid_1_luid_x_phys_0_eng_0_engtype_3d".into(), 30.0),
            ("pid_2_luid_x_phys_0_eng_0_engtype_3d".into(), 20.0),
            ("pid_3_luid_x_phys_0_eng_1_engtype_copy".into(), 40.0),
            ("pid_1_luid_y_phys_1_eng_0_engtype_3d".into(), 99.0),
        ];
        assert_eq!(aggregate_engine_usage("luid_x", &values), Some(50.0));
        assert_eq!(aggregate_engine_usage("missing", &values), None);
    }
}
