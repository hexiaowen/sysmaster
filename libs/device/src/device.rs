// Copyright (c) 2022 Huawei Technologies Co.,Ltd. All rights reserved.
//
// sysMaster is licensed under Mulan PSL v2.
// You can use this software according to the terms and conditions of the Mulan
// PSL v2.
// You may obtain a copy of Mulan PSL v2 at:
//         http://license.coscl.org.cn/MulanPSL2
// THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
// KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
// NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
// See the Mulan PSL v2 for more details.

//! struct Device
//!
use basic::devnum_util::device_path_parse_major_minor;
use libc::{dev_t, mode_t, S_IFBLK, S_IFCHR, S_IFMT};
use nix::errno::Errno;
use nix::sys::stat::{major, makedev, minor, stat};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::result::Result;
use std::sync::{Arc, Mutex};

use crate::{error::Error, DeviceAction};

/// Device
#[derive(Debug, Clone)]
pub struct Device {
    /// inotify handler
    pub watch_handle: i32,
    /// the parent device
    pub parent: Option<Arc<Mutex<Device>>>,
    /// ifindex
    pub ifindex: i32,
    /// device type
    pub devtype: String,
    /// device name
    pub devname: String,
    /// device number
    pub devnum: u64,
    /// syspath with /sys/ as prefix, e.g., /sys/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda
    pub syspath: String,
    /// relative path under /sys/, e.g., /devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda
    pub devpath: String,
    /// sysnum
    pub sysnum: Option<String>,
    /// sysname is the basename of syspath, e.g., sda
    pub sysname: String,
    /// device subsystem
    pub subsystem: String,
    /// only set for the 'drivers' subsystem
    pub driver_subsystem: String,
    /// device driver
    pub driver: String,
    /// device id
    pub device_id: String,
    /// device initialized usec
    pub usec_initialized: u64,
    /// device mode
    pub devmode: mode_t,
    /// device user id
    pub devuid: u32,
    /// device group id
    pub devgid: u32,
    // only set when device is passed through netlink
    /// uevent action
    pub action: Option<DeviceAction>,
    /// uevent seqnum
    pub seqnum: Option<u64>,
    // pub synth_uuid: u64,
    // pub partn: u32,
    /// device properties
    pub properties: HashMap<String, String>,
    /// the subset of properties that should be written to db
    pub properties_db: HashMap<String, String>,
    /// the string of properties
    pub properties_nulstr: Vec<u8>,
    /// the length of properties nulstr
    pub properties_nulstr_len: usize,
    /// cached sysattr values
    pub sysattr_values: HashMap<String, String>,
    /// names of sysattrs
    pub sysattrs: HashSet<String>,
    /// all tags
    pub all_tags: HashSet<String>,
    /// current tags
    pub current_tags: HashSet<String>,
    /// device links
    pub devlinks: HashSet<String>,
    /// block device sequence number, monothonically incremented by the kernel on create/attach
    pub diskseq: u64,

    /// whether self.properties is just now updated
    pub properties_buf_outdated: bool,
    /// whether the device is initialized by reading uevent file
    pub uevent_loaded: bool,
    /// whether the subsystem is initialized
    pub subsystem_set: bool,
    /// whether the parent is set
    pub parent_set: bool,
}

impl Default for Device {
    fn default() -> Self {
        Self::new()
    }
}

/// public methods
impl Device {
    /// create Device instance
    pub fn new() -> Device {
        Device {
            watch_handle: -1,
            ifindex: 0,
            devtype: String::new(),
            devname: String::new(),
            devnum: 0,
            syspath: String::new(),
            devpath: String::new(),
            sysnum: None,
            sysname: String::new(),
            subsystem: String::new(),
            driver_subsystem: String::new(),
            driver: String::new(),
            device_id: String::new(),
            usec_initialized: 0,
            devmode: mode_t::MAX,
            devuid: std::u32::MAX,
            devgid: std::u32::MAX,
            action: None,
            seqnum: None,
            properties: HashMap::new(),
            properties_db: HashMap::new(),
            properties_nulstr: vec![],
            properties_nulstr_len: 0,
            sysattr_values: HashMap::new(),
            sysattrs: HashSet::new(),
            all_tags: HashSet::new(),
            current_tags: HashSet::new(),
            devlinks: HashSet::new(),
            properties_buf_outdated: false,
            uevent_loaded: false,
            subsystem_set: false,
            diskseq: 0,
            parent: None,
            parent_set: false,
        }
    }

    /// create Device from buffer
    pub fn from_nulstr(nulstr: &[u8]) -> Result<Device, Error> {
        let mut device = Device::new();
        let s = std::str::from_utf8(nulstr).unwrap();
        let mut length = 0;
        let mut major = String::new();
        let mut minor = String::new();
        for line in s.split('\0') {
            let tokens = line.split('=').collect::<Vec<&str>>();
            if tokens.len() < 2 {
                break;
            }
            length = length + line.len() + 1;
            let (key, value) = (tokens[0], tokens[1]);
            match key {
                "DEVPATH" => device.set_syspath("/sys".to_string() + value, false)?,
                "ACTION" => device.set_action_from_string(value.to_string())?,
                "SUBSYSTEM" => device.set_subsystem(value.to_string())?,
                "DEVTYPE" => device.set_devtype(value.to_string())?,
                "MINOR" => minor = value.to_string(),
                "MAJOR" => major = value.to_string(),
                "DEVNAME" => device.set_devname(value.to_string())?,
                "SEQNUM" => device.set_seqnum_from_string(value.to_string())?,
                // "PARTN" => {}
                // "SYNTH_UUID" => {}
                // "USEC_INITIALIZED" => {}
                // "DRIVER" => {}
                // "IFINDEX" => {}
                // "DEVMODE" => {}
                // "DEVUID" => {}
                // "DEVGUID" => {}
                // "DISKSEQ" => {}
                // "DEVLINKS" => {}
                "TAGS" | "CURRENT_TAGS" => {}
                _ => {
                    device.add_property_internal(key.to_string(), value.to_string())?;
                }
            }
        }

        if !major.is_empty() {
            device.set_devnum(major, minor)?;
        }

        device.update_properties_bufs()?;

        Ok(device)
    }

    /// get the seqnum of Device
    pub fn get_seqnum(&self) -> Option<u64> {
        self.seqnum
    }

    /// create a Device instance based on mode and devnum
    pub fn from_mode_and_devnum(mode: mode_t, devnum: dev_t) -> Result<Device, Error> {
        let t: &str = if (mode & S_IFMT) == S_IFCHR {
            "char"
        } else if (mode & S_IFMT) == S_IFBLK {
            "block"
        } else {
            return Err(Error::Nix {
                msg: "invalid mode".to_string(),
                source: Errno::ENOTTY,
            });
        };

        if major(devnum) == 0 {
            return Err(Error::Nix {
                msg: "invalid devnum".to_string(),
                source: Errno::ENODEV,
            });
        }

        let syspath = format!("/sys/dev/{}/{}:{}", t, major(devnum), minor(devnum));

        let mut device = Device::default();
        device.set_syspath(syspath, true)?;

        // verify devnum
        let devnum_ret = device.get_devnum()?;
        if devnum_ret != devnum {
            return Err(Error::Nix {
                msg: "return inconsistent devnum".to_string(),
                source: Errno::EINVAL,
            });
        }

        // verify subsystem
        let subsystem_ret = device.get_subsystem().map_err(|e| Error::Nix {
            msg: format!(
                "from_mode_and_devnum failed: failed to verify subsystem ({})",
                e
            ),
            source: e.get_errno(),
        })?;
        if (subsystem_ret == "block") != ((mode & S_IFMT) == S_IFBLK) {
            return Err(Error::Nix {
                msg: "return inconsistent subsystem".to_string(),
                source: Errno::EINVAL,
            });
        }

        Result::Ok(device)
    }

    /// create a Device instance from devname
    /// e.g. /dev/block/8:0
    /// e.g. /dev/char/7:0
    /// e.g. /dev/sda
    pub fn from_devname(devname: String) -> Result<Device, Error> {
        if !devname.starts_with("/dev") {
            return Err(Error::Nix {
                msg: format!("the devname does not start with /dev {devname}"),
                source: Errno::EINVAL,
            });
        }

        let device = if let Ok((mode, devnum)) = device_path_parse_major_minor(devname.clone()) {
            Device::from_mode_and_devnum(mode, devnum)?
        } else {
            match stat(Path::new(&devname)) {
                Ok(st) => Device::from_mode_and_devnum(st.st_mode, st.st_rdev)?,
                Err(e) => {
                    return Err(Error::Nix {
                        msg: format!("syscall stat failed: {devname}"),
                        source: e,
                    });
                }
            }
        };

        Ok(device)
    }

    /// create a Device instance from syspath
    pub fn from_syspath(syspath: String, strict: bool) -> Result<Device, Error> {
        if strict && !syspath.starts_with("/sys/") {
            return Err(Error::Nix {
                msg: format!(
                    "from_syspath failed: syspath {} doesn't start with /sys",
                    syspath
                ),
                source: nix::errno::Errno::EINVAL,
            });
        }

        let mut device = Device::default();
        device.set_syspath(syspath, true)?;

        Ok(device)
    }

    /// create a Device instance from path
    /// path falls into two kinds: devname (/dev/...) and syspath (/sys/devices/...)
    pub fn from_path(path: String) -> Result<Device, Error> {
        if path.starts_with("/dev") {
            return Device::from_devname(path);
        }

        Device::from_syspath(path, false)
    }

    /// set sysattr value
    pub fn set_sysattr_value(
        &mut self,
        sysattr: String,
        value: Option<String>,
    ) -> Result<(), Error> {
        if value.is_none() {
            self.remove_cached_sysattr_value(sysattr)?;
            return Ok(());
        }

        let sysattr_path = self.syspath.clone() + "/" + sysattr.as_str();

        let mut file = match OpenOptions::new().write(true).open(sysattr_path.clone()) {
            Ok(f) => f,
            Err(e) => {
                return Err(Error::Nix {
                    msg: format!("failed to open sysattr file {}", sysattr_path),
                    source: Errno::from_i32(e.raw_os_error().unwrap_or_default()),
                })
            }
        };

        if let Err(e) = file.write(value.clone().unwrap().as_bytes()) {
            self.remove_cached_sysattr_value(sysattr)?;
            return Err(Error::Nix {
                msg: format!("failed to write sysattr file {}", sysattr_path),
                source: Errno::from_i32(e.raw_os_error().unwrap_or_default()),
            });
        };

        if sysattr == "uevent" {
            return Ok(());
        }

        self.cache_sysattr_value(sysattr, value.unwrap())?;

        Ok(())
    }

    /// trigger a fake device action, then kernel will report an uevent
    pub fn trigger(&mut self, action: DeviceAction) -> Result<(), Error> {
        self.set_sysattr_value("uevent".to_string(), Some(format!("{}", action)))
    }

    /// get the syspath of the device
    pub fn get_syspath(&self) -> Option<&str> {
        if self.syspath.is_empty() {
            return None;
        }

        Some(&self.syspath)
    }

    /// get the devpath of the device
    pub fn get_devpath(&self) -> Option<&str> {
        if self.devpath.is_empty() {
            return None;
        }

        Some(&self.devpath)
    }

    /// get the sysname of the device
    pub fn get_sysname(&mut self) -> Option<&str> {
        if self.sysname.is_empty() && self.set_sysname_and_sysnum().is_err() {
            log::error!("device failed to set sysname and sysnum {}", self.devpath);
            return None;
        }

        Some(&self.sysname)
    }

    /// get the parent of the device
    pub fn get_parent(&mut self) -> Result<Arc<Mutex<Device>>, Error> {
        if !self.parent_set {
            match Device::new_from_child(self) {
                Ok(parent) => self.parent = Some(Arc::new(Mutex::new(parent))),
                Err(e) => {
                    // it is okay if no parent device is found,
                    if e.get_errno() != Errno::ENODEV {
                        return Err(Error::Nix {
                            msg: format!("get parent failed because ({})", e),
                            source: e.get_errno(),
                        });
                    }
                }
            };
            self.parent_set = true;
        }

        if self.parent.is_none() {
            return Err(Error::Nix {
                msg: format!("device {} has no parent", self.devpath),
                source: Errno::ENOENT,
            });
        }

        return Ok(self.parent.as_ref().unwrap().clone());
    }
}

/// internal methods
impl Device {
    /// set the syspath of Device
    /// constraint: path should start with /sys
    pub(crate) fn set_syspath(&mut self, path: String, verify: bool) -> Result<(), Error> {
        let p = if verify {
            let path = match fs::canonicalize(path.clone()) {
                Ok(pathbuf) => pathbuf,
                Err(e) => {
                    return Err(Error::Nix {
                        msg: format!("set_syspath failed: failed to canonicalize {}", path),
                        source: Errno::from_i32(e.raw_os_error().unwrap_or_default()),
                    });
                }
            };

            if !path.starts_with("/sys") {
                // todo: what if sysfs is mounted on somewhere else?
                // systemd has considered this situation
                return Err(Error::Nix {
                    msg: format!("set_syspath failed: {:?} does not start with /sys", path),
                    source: Errno::EINVAL,
                });
            }

            if path.starts_with("/sys/devices/") {
                if !path.is_dir() {
                    return Err(Error::Nix {
                        msg: format!("set_syspath failed: {:?} is not a directory", path),
                        source: Errno::ENOTDIR,
                    });
                }

                let uevent_path = path.join("uevent");
                if !uevent_path.exists() {
                    return Err(Error::Nix {
                        msg: format!("set_syspath failed: {:?} does not contain uevent", path),
                        source: Errno::ENOENT,
                    });
                }
            } else if !path.is_dir() {
                return Err(Error::Nix {
                    msg: format!("set_syspath failed: {:?} is not a directory", path),
                    source: Errno::ENODEV,
                });
            }

            // refuse going down into /sys/fs/cgroup/ or similar places
            // where things are not arranged as kobjects in kernel

            match path.as_os_str().to_str() {
                Some(s) => s.to_string(),
                None => {
                    return Err(Error::Nix {
                        msg: format!("set_syspath failed: {:?} can not change to string", path),
                        source: Errno::EINVAL,
                    });
                }
            }
        } else {
            if !path.starts_with("/sys/") {
                return Err(Error::Nix {
                    msg: format!("set_syspath failed: {:?} does not start with /sys", path),
                    source: Errno::EINVAL,
                });
            }

            path
        };

        let devpath = match p.strip_prefix("/sys") {
            Some(p) => p,
            None => {
                return Err(Error::Nix {
                    msg: format!("set_syspath failed: syspath {} does not start with /sys", p),
                    source: Errno::EINVAL,
                });
            }
        };

        if !devpath.starts_with('/') {
            return Err(Error::Nix {
                msg: format!(
                    "set_syspath failed: devpath {} alone is not a valid device path",
                    p
                ),
                source: Errno::ENODEV,
            });
        }

        match self.add_property_internal("DEVPATH".to_string(), devpath.to_string()) {
            Ok(_) => {}
            Err(e) => {
                return Err(Error::Nix {
                    msg: format!("set_syspath failed: ({})", e),
                    source: Errno::ENODEV,
                })
            }
        }
        self.syspath = p.clone();
        self.devpath = String::from(devpath);

        Ok(())
    }

    pub(crate) fn set_sysname_and_sysnum(&mut self) -> Result<(), Error> {
        let sysname = match self.devpath.rfind('/') {
            Some(i) => String::from(&self.devpath[i + 1..]),
            None => {
                return Err(Error::Nix {
                    msg: format!(
                        "set_sysname_and_sysnum failed: invalid devpath {}",
                        self.devpath
                    ),
                    source: Errno::EINVAL,
                });
            }
        };

        let sysname = sysname.replace('!', "/");

        let mut ridx = sysname.len();
        loop {
            ridx = match sysname[0..ridx].rfind(char::is_numeric) {
                Some(ridx) => ridx,
                None => break,
            }
        }

        if ridx == sysname.len() {
            self.sysnum = None;
        } else {
            self.sysnum = Some(String::from(&sysname[ridx..]));
        }

        self.sysname = sysname;
        Ok(())
    }

    /// add property internal, in other words, do not write to external db
    pub(crate) fn add_property_internal(
        &mut self,
        key: String,
        value: String,
    ) -> Result<(), Error> {
        self.add_property_aux(key, value, false)
    }

    /// add property,
    /// if flag db is true, write to self.properties_db,
    /// else write to self.properties, and set self.properties_buf_outdated to true for updating
    pub(crate) fn add_property_aux(
        &mut self,
        key: String,
        value: String,
        db: bool,
    ) -> Result<(), Error> {
        if key.is_empty() {
            return Err(Error::Nix {
                msg: "invalid key".to_string(),
                source: Errno::EINVAL,
            });
        }

        let reference = if db {
            &mut self.properties_db
        } else {
            &mut self.properties
        };

        if value.is_empty() {
            reference.remove(&key);
        } else {
            reference.insert(key, value);
        }

        if !db {
            self.properties_buf_outdated = true;
        }

        Ok(())
    }

    /// get devnum
    pub(crate) fn get_devnum(&mut self) -> Result<u64, Error> {
        match self.read_uevent_file() {
            Ok(_) => {}
            Err(e) => {
                return Err(e);
            }
        }

        if major(self.devnum) == 0 {
            return Err(Error::Nix {
                msg: "the devnum does not exist in uevent file".to_string(),
                source: Errno::ENOENT,
            });
        }

        Ok(self.devnum)
    }

    /// get subsystem
    pub(crate) fn get_subsystem(&mut self) -> Result<&str, Error> {
        if !self.subsystem_set {
            let subsystem_path = self.syspath.clone() + "/subsystem";
            let subsystem_path = Path::new(subsystem_path.as_str());

            // get the base name of absolute subsystem path
            // e.g. /sys/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda/subsystem -> ../../../../../../../../class/block
            // get `block`
            let filename = if Path::exists(Path::new(subsystem_path)) {
                let abs_path = match fs::canonicalize(subsystem_path) {
                    Ok(ret) => ret,
                    Err(e) => {
                        return Err(Error::Nix {
                            msg: format!(
                                "get_subsystem failed: canonicalize {:?} ({})",
                                subsystem_path, e
                            ),
                            source: Errno::from_i32(e.raw_os_error().unwrap_or_default()),
                        });
                    }
                };

                abs_path.file_name().unwrap().to_str().unwrap().to_string()
            } else {
                "".to_string()
            };

            if !filename.is_empty() {
                self.set_subsystem(filename)?;
            } else if self.devpath.starts_with("/module/") {
                self.set_subsystem("module".to_string())?;
            } else if self.devpath.contains("/drivers/") || self.devpath.contains("/drivers") {
                self.set_drivers_subsystem()?;
            } else if self.devpath.starts_with("/class/") || self.devpath.starts_with("/bus/") {
                self.set_subsystem("subsystem".to_string())?;
            } else {
                self.subsystem_set = true;
            }
        };

        if !self.subsystem.is_empty() {
            Ok(&self.subsystem)
        } else {
            Err(Error::Nix {
                msg: "get_subsystem failed: no available subsystem".to_string(),
                source: Errno::ENOENT,
            })
        }
    }

    /// get properties nulstr, if it is out of date, update it
    pub(crate) fn get_properties_nulstr(&mut self) -> Result<(&Vec<u8>, usize), Error> {
        self.update_properties_bufs()?;

        Ok((&self.properties_nulstr, self.properties_nulstr_len))
    }

    /// update properties buffer
    pub(crate) fn update_properties_bufs(&mut self) -> Result<(), Error> {
        if !self.properties_buf_outdated {
            return Ok(());
        }
        self.properties_nulstr.clear();
        for (k, v) in self.properties.iter() {
            unsafe {
                self.properties_nulstr.append(k.clone().as_mut_vec());
                self.properties_nulstr.append(&mut vec![b'=']);
                self.properties_nulstr.append(v.clone().as_mut_vec());
                self.properties_nulstr.append(&mut vec![0]);
            }
        }

        self.properties_nulstr_len = self.properties_nulstr.len();
        self.properties_buf_outdated = false;
        Ok(())
    }

    /// set subsystem
    pub(crate) fn set_subsystem(&mut self, subsystem: String) -> Result<(), Error> {
        self.add_property_internal("SUBSYSTEM".to_string(), subsystem.clone())?;
        self.subsystem_set = true;
        self.subsystem = subsystem;
        Ok(())
    }

    /// set drivers subsystem
    pub(crate) fn set_drivers_subsystem(&mut self) -> Result<(), Error> {
        let mut subsystem = String::new();
        let components: Vec<&str> = self.devpath.split('/').collect();
        for (idx, com) in components.iter().enumerate() {
            if *com == "drivers" {
                subsystem = components.get(idx - 1).unwrap().to_string();
                break;
            }
        }

        if subsystem.is_empty() {
            return Err(Error::Nix {
                msg: "invalid driver subsystem".to_string(),
                source: Errno::EINVAL,
            });
        }

        self.set_subsystem(subsystem.clone())?;
        self.driver_subsystem = subsystem;

        Ok(())
    }

    /// read uevent file and filling device attributes
    pub(crate) fn read_uevent_file(&mut self) -> Result<(), Error> {
        if self.uevent_loaded {
            return Ok(());
        }

        let uevent_file = self.syspath.clone() + "/uevent";

        let mut file = match fs::OpenOptions::new().read(true).open(uevent_file) {
            Ok(f) => f,
            Err(e) => match e.raw_os_error() {
                Some(n) => {
                    return Err(Error::Nix {
                        msg: "failed to open uevent file".to_string(),
                        source: Errno::from_i32(n),
                    });
                }
                None => {
                    return Err(Error::Nix {
                        msg: "failed to open uevent file".to_string(),
                        source: Errno::EINVAL,
                    });
                }
            },
        };

        let mut buf = String::new();
        file.read_to_string(&mut buf).unwrap();

        let mut major = String::new();
        let mut minor = String::new();

        for line in buf.split('\n') {
            let tokens: Vec<&str> = line.split('=').collect();
            if tokens.len() < 2 {
                break;
            }

            let (key, value) = (tokens[0], tokens[1]);

            match key {
                "DEVTYPE" => self.set_devtype(value.to_string())?,
                "IFINDEX" => self.set_ifindex(value.to_string())?,
                "DEVNAME" => self.set_devname(value.to_string())?,
                "DEVMODE" => self.set_devmode(value.to_string())?,
                "DISKSEQ" => self.set_diskseq(value.to_string())?,
                "MAJOR" => {
                    major = value.to_string();
                }
                "MINOR" => {
                    minor = value.to_string();
                }
                _ => {}
            }
        }

        self.set_devnum(major, minor)?;

        self.uevent_loaded = true;

        Ok(())
    }

    /// set devtype
    pub(crate) fn set_devtype(&mut self, devtype: String) -> Result<(), Error> {
        self.add_property_internal("DEVTYPE".to_string(), devtype.clone())?;
        self.devtype = devtype;
        Ok(())
    }

    /// set ifindex
    pub(crate) fn set_ifindex(&mut self, ifindex: String) -> Result<(), Error> {
        self.add_property_internal("IFINDEX".to_string(), ifindex.clone())?;
        self.ifindex = match ifindex.parse() {
            Ok(idx) => idx,
            Err(e) => {
                return Err(Error::Nix {
                    msg: e.to_string(),
                    source: Errno::EINVAL,
                });
            }
        };
        Ok(())
    }

    /// set devname
    pub(crate) fn set_devname(&mut self, devname: String) -> Result<(), Error> {
        let devname = if devname.starts_with('/') {
            devname
        } else {
            "/dev/".to_string() + devname.as_str()
        };

        self.add_property_internal("DEVNAME".to_string(), devname.clone())?;
        self.devname = devname;
        Ok(())
    }

    /// set devmode
    pub(crate) fn set_devmode(&mut self, devmode: String) -> Result<(), Error> {
        self.add_property_internal("DEVMODE".to_string(), devmode.clone())?;

        self.devmode = match devmode.parse() {
            Ok(m) => m,
            Err(e) => {
                return Err(Error::Nix {
                    msg: e.to_string(),
                    source: Errno::EINVAL,
                });
            }
        };

        Ok(())
    }

    /// set devnum
    pub(crate) fn set_devnum(&mut self, major: String, minor: String) -> Result<(), Error> {
        let major_num: u64 = match major.parse() {
            Ok(n) => n,
            Err(e) => {
                return Err(Error::Nix {
                    msg: e.to_string(),
                    source: Errno::EINVAL,
                });
            }
        };
        let minor_num: u64 = match minor.parse() {
            Ok(n) => n,
            Err(e) => {
                return Err(Error::Nix {
                    msg: e.to_string(),
                    source: Errno::EINVAL,
                });
            }
        };

        self.add_property_internal("MAJOR".to_string(), major)?;
        self.add_property_internal("MINOR".to_string(), minor)?;
        self.devnum = makedev(major_num, minor_num);

        Ok(())
    }

    /// set diskseq
    pub(crate) fn set_diskseq(&mut self, diskseq: String) -> Result<(), Error> {
        self.add_property_internal("DISKSEQ".to_string(), diskseq.clone())?;

        let diskseq_num: u64 = match diskseq.parse() {
            Ok(n) => n,
            Err(e) => {
                return Err(Error::Nix {
                    msg: e.to_string(),
                    source: Errno::EINVAL,
                });
            }
        };

        self.diskseq = diskseq_num;

        Ok(())
    }

    /// set action
    pub(crate) fn set_action(&mut self, action: DeviceAction) -> Result<(), Error> {
        self.add_property_internal("ACTION".to_string(), action.to_string())?;
        self.action = Some(action);
        Ok(())
    }

    /// set action from string
    pub(crate) fn set_action_from_string(&mut self, action_s: String) -> Result<(), Error> {
        let action = match action_s.parse::<DeviceAction>() {
            Ok(a) => a,
            Err(_) => {
                return Err(Error::Nix {
                    msg: format!("invalid action string {}", action_s),
                    source: Errno::EINVAL,
                });
            }
        };

        self.set_action(action)
    }

    /// set seqnum from string
    pub(crate) fn set_seqnum_from_string(&mut self, seqnum_s: String) -> Result<(), Error> {
        let seqnum: u64 = match seqnum_s.parse() {
            Ok(n) => n,
            Err(_) => {
                return Err(Error::Nix {
                    msg: format!("invalid seqnum can not be parsed to u64 {}", seqnum_s),
                    source: Errno::EINVAL,
                });
            }
        };

        self.set_seqnum(seqnum)
    }

    /// set seqnum
    pub(crate) fn set_seqnum(&mut self, seqnum: u64) -> Result<(), Error> {
        self.add_property_internal("SEQNUM".to_string(), seqnum.to_string())?;
        self.seqnum = Some(seqnum);
        Ok(())
    }

    /// cache sysattr value
    pub(crate) fn cache_sysattr_value(
        &mut self,
        sysattr: String,
        value: String,
    ) -> Result<(), Error> {
        self.sysattr_values.insert(sysattr, value);

        Ok(())
    }

    /// remove cached sysattr value
    pub(crate) fn remove_cached_sysattr_value(&mut self, sysattr: String) -> Result<(), Error> {
        self.sysattr_values.remove(&sysattr);

        Ok(())
    }

    /// new from child
    pub(crate) fn new_from_child(device: &mut Device) -> Result<Device, Error> {
        let syspath = match device.get_syspath() {
            Some(ret) => Path::new(ret),
            None => {
                return Err(Error::Nix {
                    msg: "new_from_child failed: can not get syspath".to_string(),
                    source: Errno::EINVAL,
                });
            }
        };

        let mut parent = syspath.parent();

        loop {
            match parent {
                Some(p) => {
                    if p == Path::new("/sys") {
                        return Err(Error::Nix {
                            msg: "new_from_child failed: no available parent device until /sys"
                                .to_string(),
                            source: Errno::ENODEV,
                        });
                    }
                    let path = p.to_str().unwrap().to_string();

                    match Device::from_syspath(path, true) {
                        Ok(d) => return Ok(d),
                        Err(e) => {
                            if e.get_errno() != Errno::ENODEV {
                                return Err(Error::Nix {
                                    msg: format!(
                                        "new_from_child failed: from_syspath failed ({})",
                                        e
                                    ),
                                    source: e.get_errno(),
                                });
                            }
                        }
                    }
                }
                None => {
                    return Err(Error::Nix {
                        msg: "new_from_child failed: no available parent device".to_string(),
                        source: Errno::ENODEV,
                    });
                }
            }

            parent = parent.unwrap().parent();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::device::*;
    use libc::S_IFBLK;
    use nix::sys::stat::makedev;

    /// test whether Device::from_mode_and_devnum can create Device instance normally
    #[ignore]
    #[test]
    fn test_from_mode_and_devnum() {
        let devnum = makedev(8, 0);
        let mode = S_IFBLK;
        let device = Device::from_mode_and_devnum(mode, devnum).unwrap();

        assert_eq!(
            "/sys/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda",
            device.syspath
        );
        assert_eq!(
            "/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda",
            device.devpath
        );
        assert_eq!("block", device.subsystem);
        assert_eq!(makedev(8, 0), device.devnum);
        assert_eq!("/dev/sda", device.devname);
    }

    /// test whether Device::from_devname can create Device instance normally
    #[ignore]
    #[test]
    fn test_from_devname() {
        let devname = "/dev/sda".to_string();
        let device = Device::from_devname(devname).unwrap();

        assert_eq!(
            "/sys/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda",
            device.syspath
        );
        assert_eq!(
            "/devices/pci0000:00/0000:00:10.0/host2/target2:0:1/2:0:1:0/block/sda",
            device.devpath
        );
        assert_eq!("block", device.subsystem);
        assert_eq!("/dev/sda", device.devname);
    }

    /// test whether Device::set_sysattr_value can work normally
    #[ignore]
    #[test]
    fn test_set_sysattr_value() {
        let devname = "/dev/sda".to_string();
        let mut device = Device::from_devname(devname).unwrap();

        device
            .set_sysattr_value("uevent".to_string(), Some("change".to_string()))
            .unwrap();
    }
}
