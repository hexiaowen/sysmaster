//! Monitor the number of process fds
use serde_derive::Deserialize;

use libutils::Error;
use std::fs::OpenOptions;
use std::io::Write;

use crate::{Monitor, Switch, SysMonitor};

const CONFIG_FILE_PATH: &str = "/etc/sysmonitor/process_fd_conf";
const PROC_FDTHRESHOLD: &str = "/proc/fdthreshold";
const PROC_FDENABLE: &str = "/proc/fdenable";

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "UPPERCASE")]
pub struct ProcessFd {
    pub(crate) config: Switch,
    #[serde(default = "alarm_default")]
    pub alarm: u32,
}

fn alarm_default() -> u32 {
    80
}

impl Monitor for ProcessFd {
    fn config_path(&self) -> &str {
        CONFIG_FILE_PATH
    }

    fn load(&mut self, content: String, sysmonitor: SysMonitor) {
        let monitor: Self = toml::from_str(content.as_str()).unwrap();
        *self = ProcessFd {
            config: Switch {
                monitor: sysmonitor.process_fd_num_monitor,
                alarm: sysmonitor.process_fd_num_alarm,
            },
            ..monitor
        };
    }

    fn is_valid(&self) -> bool {
        self.alarm > 0 && self.alarm < 100
    }

    fn check_status(&mut self) -> Result<(), Error> {
        // Write the value to procfs, turn on monitoring, the real monitoring is implemented by the kernel
        write_file(PROC_FDTHRESHOLD, self.alarm.to_string())?;
        write_file(PROC_FDENABLE, 1.to_string())?;
        Ok(())
    }

    fn report_alarm(&self) {}
}

fn write_file(path: &str, str: String) -> Result<(), std::io::Error> {
    let mut f = OpenOptions::new().read(false).write(true).open(path)?;
    f.write_all(str.as_bytes().as_ref())
}
