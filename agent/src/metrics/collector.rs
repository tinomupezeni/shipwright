use sysinfo::{System, CpuRefreshKind, RefreshKind, MemoryRefreshKind};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
}

pub struct Collector {
    sys: System,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            sys: System::new_with_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            ),
        }
    }

    pub fn collect(&mut self) -> SystemMetrics {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        SystemMetrics {
            cpu_usage: self.sys.global_cpu_info().cpu_usage(),
            memory_used: self.sys.used_memory(),
            memory_total: self.sys.total_memory(),
        }
    }
}
