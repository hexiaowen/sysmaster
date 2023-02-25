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

//! socket_mng is the core of the socket unit，implement the state transition, ports management and sub child management.
//!
use super::{
    comm::SocketUnitComm,
    config::SocketConfig,
    pid::SocketPid,
    port::SocketPort,
    rentry::{PortType, SocketCommand, SocketRe, SocketReFrame, SocketResult, SocketState},
    spawn::SocketSpawn,
};
use libevent::EventState;
use libevent::{EventType, Events, Source};
use libutils::IN_SET;
use nix::libc::{self};
use nix::{errno::Errno, sys::wait::WaitStatus};
use std::cell::RefCell;
use std::os::unix::prelude::RawFd;
use std::rc::{Rc, Weak};
use sysmaster::error::*;
use sysmaster::exec::{ExecCommand, ExecContext};
use sysmaster::rel::ReliLastFrame;
use sysmaster::rel::{ReStation, Reliability};
use sysmaster::unit::{KillOperation, UnitActiveState, UnitNotifyFlags, UnitType};

impl SocketState {
    pub(super) fn to_unit_active_state(self) -> UnitActiveState {
        match self {
            SocketState::Dead => UnitActiveState::UnitInActive,
            SocketState::StartPre | SocketState::StartChown | SocketState::StartPost => {
                UnitActiveState::UnitActivating
            }
            SocketState::Listening | SocketState::Running => UnitActiveState::UnitActive,
            SocketState::StopPre
            | SocketState::StopPreSigterm
            | SocketState::StopPost
            | SocketState::StopPreSigkill
            | SocketState::StateMax
            | SocketState::FinalSigterm
            | SocketState::FinalSigkill => UnitActiveState::UnitDeActivating,
            SocketState::Failed => UnitActiveState::UnitFailed,
            SocketState::Cleaning => UnitActiveState::UnitMaintenance,
        }
    }

    fn to_kill_operation(self) -> KillOperation {
        match self {
            SocketState::StopPreSigterm => {
                // todo!() check has a restart job
                KillOperation::KillKill
            }
            SocketState::FinalSigterm => KillOperation::KillTerminate,
            _ => KillOperation::KillKill,
        }
    }
}

pub(super) struct SocketMng {
    data: Rc<SocketMngData>,
}

impl ReStation for SocketMng {
    // input: do nothing

    // compensate: do nothing

    // data
    fn db_map(&self) {
        self.build_ports();
        self.data.db_map();
    }

    fn db_insert(&self) {
        self.data.db_insert();
    }

    // reload: entry-only
    fn entry_coldplug(&self) {
        self.data.entry_coldplug();
    }

    fn entry_clear(&self) {
        self.data.entry_clear();
    }
}

impl SocketMng {
    pub(super) fn new(
        commr: &Rc<SocketUnitComm>,
        configr: &Rc<SocketConfig>,
        exec_ctx: &Rc<ExecContext>,
    ) -> SocketMng {
        SocketMng {
            data: SocketMngData::new(commr, configr, exec_ctx),
        }
    }

    pub(super) fn start_check(&self) -> Result<bool> {
        self.data.start_check()
    }

    pub(super) fn start_action(&self) {
        self.data.start_action();
        self.db_update();
    }

    pub(super) fn stop_check(&self) -> Result<bool> {
        self.data.stop_check()
    }

    pub(super) fn stop_action(&self) {
        self.data.stop_action();
        self.db_update();
    }

    pub(super) fn get_state(&self) -> String {
        let state = self.data.state();
        state.to_string()
    }

    pub(super) fn sigchld_event(&self, wait_status: WaitStatus) {
        self.data.sigchld_event(wait_status);
        self.db_update();
    }

    pub(super) fn current_active_state(&self) -> UnitActiveState {
        self.data.current_active_state()
    }

    pub(super) fn collect_fds(&self) -> Vec<i32> {
        self.data.collect_fds()
    }

    pub(super) fn build_ports(&self) {
        self.data.build_ports(&self.data);
        self.db_update();
    }
}

struct SocketMngData {
    // associated objects
    comm: Rc<SocketUnitComm>,
    config: Rc<SocketConfig>,

    // owned objects
    pid: SocketPid,
    spawn: SocketSpawn,
    ports: RefCell<Vec<Rc<SocketMngPort>>>,
    state: Rc<RefCell<SocketState>>,
    result: RefCell<SocketResult>,
    control_cmd_type: RefCell<Option<SocketCommand>>,
    control_command: RefCell<Vec<ExecCommand>>,
    refused: RefCell<i32>,
}

// the declaration "pub(self)" is for identification only.
impl SocketMngData {
    pub(self) fn new(
        commr: &Rc<SocketUnitComm>,
        configr: &Rc<SocketConfig>,
        exec_ctx: &Rc<ExecContext>,
    ) -> Rc<SocketMngData> {
        Rc::new(SocketMngData {
            comm: Rc::clone(commr),
            config: Rc::clone(configr),

            pid: SocketPid::new(commr),
            spawn: SocketSpawn::new(commr, exec_ctx),
            ports: RefCell::new(Vec::new()),
            state: Rc::new(RefCell::new(SocketState::StateMax)),
            result: RefCell::new(SocketResult::Success),
            control_cmd_type: RefCell::new(None),
            control_command: RefCell::new(Vec::new()),
            refused: RefCell::new(0),
        })
    }

    pub(self) fn db_map(&self) {
        if let Some((state, result, c_pid, control_cmd_type, control_cmd_len, refused, rports)) =
            self.comm.rentry_mng_get()
        {
            *self.state.borrow_mut() = state;
            *self.result.borrow_mut() = result;
            self.pid.update_control(c_pid);
            self.control_command_update(control_cmd_type, control_cmd_len);
            *self.refused.borrow_mut() = refused;
            self.map_ports_fd(rports);
        }
    }

    fn entry_clear(&self) {
        self.unwatch_fds();
        // self.unwatch_pid_file: todo!()
    }

    fn entry_coldplug(&self) {
        self.watch_fds();
    }

    pub(self) fn start_check(&self) -> Result<bool> {
        if IN_SET!(
            self.state(),
            SocketState::StopPre,
            SocketState::StopPreSigkill,
            SocketState::StopPreSigterm,
            SocketState::StopPost,
            SocketState::FinalSigterm,
            SocketState::FinalSigkill,
            SocketState::Cleaning
        ) {
            return Err(Error::UnitActionEAgain);
        }

        if IN_SET!(
            self.state(),
            SocketState::StartPre,
            SocketState::StartChown,
            SocketState::StartPost
        ) {
            return Ok(true);
        }

        self.config.unit_ref_target().map_or(Ok(()), |name| {
            match self.comm.um().unit_enabled(&name) {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            }
        })?;
        let ret = self.comm.owner().map(|u| u.test_start_limit());
        if ret.is_none() || !ret.unwrap() {
            self.enter_dead(SocketResult::FailureStartLimitHit);
            return Err(Error::UnitActionECanceled);
        }
        Ok(false)
    }

    pub(self) fn start_action(&self) {
        self.enter_start_pre();
    }

    pub(self) fn stop_action(&self) {
        self.enter_stop_pre(SocketResult::Success);
    }

    pub(self) fn stop_check(&self) -> Result<bool> {
        if IN_SET!(
            self.state(),
            SocketState::StopPre,
            SocketState::StopPreSigterm,
            SocketState::StopPreSigkill,
            SocketState::StopPost,
            SocketState::FinalSigterm,
            SocketState::FinalSigkill
        ) {
            return Ok(true);
        }

        if IN_SET!(
            self.state(),
            SocketState::StartPre,
            SocketState::StartChown,
            SocketState::StartPost
        ) {
            self.enter_signal(SocketState::StopPreSigterm, SocketResult::Success);
            return Err(Error::UnitActionEAgain);
        }

        Ok(false)
    }

    pub(self) fn current_active_state(&self) -> UnitActiveState {
        self.state().to_unit_active_state()
    }

    #[allow(dead_code)]
    pub(self) fn clear_ports(&self) {
        self.ports.borrow_mut().clear();
    }

    pub(self) fn collect_fds(&self) -> Vec<i32> {
        let mut fds = Vec::new();
        for port in self.ports().iter() {
            if port.fd() >= 0 {
                fds.push(port.fd());
            }
        }

        fds
    }

    fn enter_start_pre(&self) {
        log::debug!("enter start pre command");
        self.pid.unwatch_control();

        self.control_command_fill(SocketCommand::StartPre);
        match self.control_command_pop() {
            Some(cmd) => {
                match self.spawn.start_socket(&cmd) {
                    Ok(pid) => self.pid.set_control(pid),
                    #[allow(clippy::unit_arg)]
                    Err(e) => {
                        self.comm.owner().map_or_else(
                            || {
                                log::error!(
                                "Failed to run start pre service, unit name is None,error: {:?}",
                                e
                            );
                            },
                            |u| {
                                log::error!(
                                    "Failed to run start pre service: {}, error: {:?}",
                                    u.id(),
                                    e
                                );
                            },
                        );
                        self.enter_dead(SocketResult::FailureResources);
                        return;
                    }
                }
                self.set_state(SocketState::StartPre);
            }
            None => self.enter_start_chown(),
        }
    }

    fn enter_start_chown(&self) {
        log::debug!("enter start chown command");
        match self.open_fds() {
            Ok(_) => {
                self.enter_start_post();
            }
            Err(_) => self.enter_stop_pre(SocketResult::FailureResources),
        }
    }

    fn enter_start_post(&self) {
        log::debug!("enter start post command");
        self.pid.unwatch_control();
        self.control_command_fill(SocketCommand::StartPost);

        match self.control_command_pop() {
            Some(cmd) => {
                match self.spawn.start_socket(&cmd) {
                    Ok(pid) => self.pid.set_control(pid),
                    Err(_e) => {
                        if let Some(u) = self.comm.owner() {
                            log::error!("Failed to run start post service: {}", u.id());
                        } else {
                            log::error!("Failed to run start post service unit id is None");
                        }
                        self.enter_stop_pre(SocketResult::FailureResources);
                        return;
                    }
                }
                self.set_state(SocketState::StartPost);
            }
            None => self.enter_listening(),
        }
    }

    fn enter_listening(&self) {
        log::debug!("enter start listening state");
        if !self.config.config_data().borrow().Socket.Accept {
            self.flush_ports();
        }

        self.watch_fds();

        self.set_state(SocketState::Listening)
    }

    fn enter_running(&self, fd: i32) {
        if let Some(u) = self.comm.owner() {
            if self.comm.um().has_stop_job(u.id()) {
                if fd >= 0 {
                    *self.refused.borrow_mut() += 1;
                    return;
                }
                self.flush_ports();
                return;
            }
            if fd < 0 {
                if !self.comm.um().relation_active_or_pending(u.id()) {
                    if self.config.unit_ref_target().is_none() {
                        self.enter_stop_pre(SocketResult::FailureResources);
                        return;
                    }
                    let service = self.config.unit_ref_target().unwrap();

                    // start corresponding *.service
                    self.rentry().set_last_frame(SocketReFrame::FdListen(false)); // protect 'start_unit'
                    let ret = self.comm.um().start_unit(&service);
                    self.rentry().set_last_frame(SocketReFrame::FdListen(true));
                    if ret.is_err() {
                        self.enter_stop_pre(SocketResult::FailureResources);
                        return;
                    }
                }
                self.set_state(SocketState::Running);
            } else {
                // template support
                todo!()
            }
        }
    }

    fn enter_stop_pre(&self, res: SocketResult) {
        log::debug!("enter stop pre command");
        if self.result() == SocketResult::Success {
            self.set_result(res);
        }

        self.pid.unwatch_control();

        self.control_command_fill(SocketCommand::StopPre);

        match self.control_command_pop() {
            Some(cmd) => {
                match self.spawn.start_socket(&cmd) {
                    Ok(pid) => self.pid.set_control(pid),
                    Err(_e) => {
                        if let Some(u) = self.comm.owner() {
                            log::error!("Failed to run stop pre cmd for service: {}", u.id());
                        } else {
                            log::error!("Failed to run stop pre cmd and service unit id is None");
                        }
                        self.enter_stop_post(SocketResult::FailureResources);
                        return;
                    }
                }
                self.set_state(SocketState::StopPre);
            }
            None => self.enter_stop_post(SocketResult::Success),
        }
    }

    fn enter_stop_post(&self, res: SocketResult) {
        log::debug!("enter stop post command");
        if self.result() == SocketResult::Success {
            self.set_result(res);
        }

        self.control_command_fill(SocketCommand::StopPost);

        match self.control_command_pop() {
            Some(cmd) => {
                match self.spawn.start_socket(&cmd) {
                    Ok(pid) => self.pid.set_control(pid),
                    Err(e) => {
                        #[allow(clippy::unit_arg)]
                        self.comm.owner().map_or(
                            log::error!("Failed to run stop post cmd and service unit id is None"),
                            |u| {
                                log::error!(
                                    "Failed to run stop post cmd for service: {},err {}",
                                    u.id(),
                                    e
                                )
                            },
                        );
                        self.enter_signal(
                            SocketState::FinalSigterm,
                            SocketResult::FailureResources,
                        );
                        return;
                    }
                }
                self.set_state(SocketState::StopPost);
            }
            None => self.enter_signal(SocketState::FinalSigterm, SocketResult::Success),
        }
    }

    fn enter_signal(&self, state: SocketState, res: SocketResult) {
        log::debug!("enter enter signal {:?}, res: {:?}", state, res);
        if self.result() == SocketResult::Success {
            self.set_result(res);
        }

        let op = state.to_kill_operation();
        if let Some(u) = self.comm.owner() {
            match u.kill_context(self.config.kill_context(), None, self.pid.control(), op) {
                Ok(_) => {}
                Err(_e) => {
                    if IN_SET!(
                        state,
                        SocketState::StopPreSigterm,
                        SocketState::StopPreSigkill
                    ) {
                        return self.enter_stop_post(SocketResult::FailureResources);
                    } else {
                        return self.enter_dead(SocketResult::FailureResources);
                    }
                }
            }
        };

        if state == SocketState::StopPreSigterm {
            self.enter_signal(SocketState::StopPreSigkill, SocketResult::Success);
        } else if state == SocketState::StopPreSigkill {
            self.enter_stop_post(SocketResult::Success);
        } else if state == SocketState::FinalSigterm {
            self.enter_signal(SocketState::FinalSigkill, SocketResult::Success);
        } else {
            self.enter_dead(SocketResult::Success)
        }
    }

    fn enter_dead(&self, res: SocketResult) {
        log::debug!("enter enter dead state, res {:?}", res);
        if self.result() == SocketResult::Success {
            self.set_result(res);
        }

        let state = if self.result() == SocketResult::Success {
            SocketState::Dead
        } else {
            SocketState::Failed
        };

        self.set_state(state);
    }

    fn run_next(&self) {
        if let Some(cmd) = self.control_command_pop() {
            match self.spawn.start_socket(&cmd) {
                Ok(pid) => self.pid.set_control(pid),
                Err(_e) => {
                    if let Some(u) = self.comm.owner() {
                        log::error!("failed to run main command unit{},err {}", u.id(), _e);
                    } else {
                        log::error!("failed to run main command unit is None,Error: {}", _e);
                    }
                }
            }
        }
    }

    fn open_fds(&self) -> Result<(), Errno> {
        for port in self.ports().iter() {
            let ret1 = port.open_port(true);
            if ret1.is_err() {
                self.close_fds();
                return ret1.map(|_| ());
            }

            port.apply_sock_opt(port.fd());
        }

        Ok(())
    }

    fn close_fds(&self) {
        // event
        let events = self.comm.um().events();
        for mport in self.mports().iter() {
            let source = Rc::clone(mport);
            events.del_source(source).unwrap();
        }

        // close
        for port in self.ports().iter() {
            port.close(true);
        }
    }

    fn watch_fds(&self) {
        let events = self.comm.um().events();
        for mport in self.mports().iter() {
            if mport.fd() < 0 {
                continue;
            }
            let source = Rc::clone(mport);
            events.add_source(source).unwrap();
            let source = Rc::clone(mport);
            events.set_enabled(source, EventState::On).unwrap();
        }
    }

    fn unwatch_fds(&self) {
        let events = self.comm.um().events();
        for mport in self.mports().iter() {
            let source = Rc::clone(mport);
            events.set_enabled(source, EventState::Off).unwrap();
        }
    }

    fn flush_ports(&self) {
        for port in self.ports().iter() {
            port.flush_accept();

            port.flush_fd();
        }
    }

    fn set_state(&self, state: SocketState) {
        let original_state = self.state();
        *self.state.borrow_mut() = state;

        // TODO
        // check the new state
        if !vec![
            SocketState::StartPre,
            SocketState::StartChown,
            SocketState::StartPost,
            SocketState::StopPre,
            SocketState::StopPreSigterm,
            SocketState::StopPreSigkill,
            SocketState::StopPost,
            SocketState::FinalSigterm,
            SocketState::FinalSigkill,
        ]
        .contains(&state)
        {
            self.pid.unwatch_control();
        }

        if state != SocketState::Listening {
            self.unwatch_fds();
        }

        if !vec![
            SocketState::StartChown,
            SocketState::StartPost,
            SocketState::Listening,
            SocketState::Running,
            SocketState::StopPre,
            SocketState::StopPreSigterm,
            SocketState::StopPreSigkill,
        ]
        .contains(&state)
        {
            self.close_fds();
        }

        log::debug!(
            "original state: {:?}, change to: {:?}",
            original_state,
            state
        );
        // todo!()
        // trigger the unit the dependency trigger_by

        if let Some(u) = self.comm.owner() {
            u.notify(
                original_state.to_unit_active_state(),
                state.to_unit_active_state(),
                UnitNotifyFlags::UNIT_NOTIFY_RELOAD_FAILURE,
            )
        }
    }

    fn state(&self) -> SocketState {
        *self.state.borrow()
    }

    fn control_command_fill(&self, cmd_type: SocketCommand) {
        if let Some(cmds) = self.config.get_exec_cmds(cmd_type) {
            *self.control_command.borrow_mut() = cmds
        }
    }

    fn control_command_pop(&self) -> Option<ExecCommand> {
        self.control_command.borrow_mut().pop()
    }

    fn control_command_update(&self, cmd_type: Option<SocketCommand>, len: usize) {
        if let Some(c_type) = cmd_type {
            self.control_command.borrow_mut().clear();
            self.control_command_fill(c_type);
            let max = self.control_command.borrow().len();
            for _i in len..max {
                self.control_command_pop();
            }
        } else {
            assert_eq!(len, 0);
        }
    }

    fn result(&self) -> SocketResult {
        *self.result.borrow()
    }

    fn set_result(&self, res: SocketResult) {
        *self.result.borrow_mut() = res;
    }

    fn build_ports(&self, mng: &Rc<SocketMngData>) {
        for p_conf in self.config.ports().iter() {
            let port = Rc::new(SocketPort::new(&self.comm, &self.config, p_conf));
            let mport = Rc::new(SocketMngPort::new(mng, port));
            self.ports.borrow_mut().push(mport);
        }
    }

    fn map_ports_fd(&self, rports: Vec<(PortType, String, RawFd)>) {
        assert_eq!(rports.len(), self.ports().len());

        for (p_type, listen, fd) in rports.iter() {
            let port = self.ports_find(*p_type, listen).unwrap();
            port.set_fd(self.comm.reli().fd_take(*fd));
        }
    }

    fn mports(&self) -> Vec<Rc<SocketMngPort>> {
        self.ports.borrow().iter().map(Rc::clone).collect::<_>()
    }

    fn ports_find(&self, p_type: PortType, listen: &str) -> Option<Rc<SocketPort>> {
        let ports = self.ports();
        for port in ports.iter() {
            if port.p_type() == p_type && port.listen() == listen {
                return Some(Rc::clone(port));
            }
        }

        None
    }

    fn ports(&self) -> Vec<Rc<SocketPort>> {
        self.ports
            .borrow()
            .iter()
            .map(|p| Rc::clone(&p.port))
            .collect::<_>()
    }

    fn rentry(&self) -> Rc<SocketRe> {
        self.comm.rentry()
    }

    fn db_insert(&self) {
        self.comm.rentry_mng_insert(
            self.state(),
            self.result(),
            self.pid.control(),
            *self.control_cmd_type.borrow(),
            self.control_command.borrow().len(),
            *self.refused.borrow(),
            self.ports()
                .iter()
                .map(|p| (p.p_type(), String::from(p.listen()), p.fd()))
                .collect::<_>(),
        );
    }

    fn db_update(&self) {
        self.db_insert();
    }
}

// the declaration "pub(self)" is for identification only.
impl SocketMngData {
    fn sigchld_result(&self, wait_status: WaitStatus) -> SocketResult {
        match wait_status {
            WaitStatus::Exited(_, status) => {
                if status == 0 {
                    SocketResult::Success
                } else {
                    SocketResult::FailureExitCode
                }
            }
            WaitStatus::Signaled(_, _, core_dump) => {
                if core_dump {
                    SocketResult::FailureCoreDump
                } else {
                    SocketResult::FailureSignal
                }
            }
            _ => unreachable!(),
        }
    }

    pub(self) fn sigchld_event(&self, wait_status: WaitStatus) {
        let res = self.sigchld_result(wait_status);

        if !self.control_command.borrow().is_empty() && res == SocketResult::Success {
            self.run_next();
        } else {
            match self.state() {
                SocketState::StartPre => {
                    if res == SocketResult::Success {
                        self.enter_start_chown();
                    } else {
                        self.enter_signal(SocketState::FinalSigterm, res);
                    }
                }
                SocketState::StartChown => {
                    if res == SocketResult::Success {
                        self.enter_start_post();
                    } else {
                        self.enter_stop_pre(res);
                    }
                }
                SocketState::StartPost => {
                    if res == SocketResult::Success {
                        self.enter_listening();
                    } else {
                        self.enter_stop_pre(res);
                    }
                }
                SocketState::StopPre
                | SocketState::StopPreSigterm
                | SocketState::StopPreSigkill => {
                    self.enter_stop_post(res);
                }
                SocketState::StopPost | SocketState::FinalSigterm | SocketState::FinalSigkill => {
                    self.enter_dead(res);
                }
                _ => {
                    log::error!(
                        "control command should not exit, current state is : {:?}",
                        self.state()
                    );
                    unreachable!();
                }
            }
        }
    }
}

struct SocketMngPort {
    // associated objects
    mng: Weak<SocketMngData>,

    // owned objects
    port: Rc<SocketPort>,
}

impl Source for SocketMngPort {
    fn fd(&self) -> RawFd {
        self.port.fd()
    }

    fn event_type(&self) -> EventType {
        EventType::Io
    }

    fn epoll_event(&self) -> u32 {
        (libc::EPOLLIN) as u32
    }

    fn priority(&self) -> i8 {
        0i8
    }

    fn dispatch(&self, _: &Events) -> i32 {
        println!("Dispatching IO!");

        self.reli().set_last_frame2(
            ReliLastFrame::SubManager as u32,
            UnitType::UnitSocket as u32,
        );
        self.rentry().set_last_frame(SocketReFrame::FdListen(true));
        self.reli()
            .set_last_unit(self.mng().comm.owner().unwrap().id());
        let ret = self.dispatch_io().map_err(|_| libevent::Error::Other {
            word: "Dispatch IO failed!",
        });
        self.reli().clear_last_unit();
        self.rentry().clear_last_frame();
        self.reli().clear_last_frame();
        ret.unwrap_or(-1)
    }

    fn token(&self) -> u64 {
        let data: u64 = unsafe { std::mem::transmute(self) };
        data
    }
}

// the declaration "pub(self)" is for identification only.
impl SocketMngPort {
    pub(self) fn new(mng: &Rc<SocketMngData>, port: Rc<SocketPort>) -> SocketMngPort {
        SocketMngPort {
            mng: Rc::downgrade(mng),
            port,
        }
    }

    fn dispatch_io(&self) -> Result<i32> {
        let afd: i32 = -1;

        if self.mng().state() != SocketState::Listening {
            return Ok(0);
        }

        if self.mng().config.config_data().borrow().Socket.Accept
            && self.port.p_type() == PortType::Socket
            && self.port.sa().can_accept()
        {
            let afd = self.port.accept().map_err(|_e| Error::Other {
                msg: "accept err".to_string(),
            })?;

            self.port.apply_sock_opt(afd)
        }

        self.mng().enter_running(afd);
        self.mng().db_update();

        Ok(0)
    }

    fn reli(&self) -> Rc<Reliability> {
        self.mng().comm.reli()
    }

    fn rentry(&self) -> Rc<SocketRe> {
        self.mng().comm.rentry()
    }

    fn mng(&self) -> Rc<SocketMngData> {
        self.mng.clone().upgrade().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::SocketState;
    use sysmaster::unit::UnitActiveState;
    #[test]
    fn test_socket_active_state() {
        assert_eq!(
            SocketState::Dead.to_unit_active_state(),
            UnitActiveState::UnitInActive
        );
        assert_eq!(
            SocketState::StartPre.to_unit_active_state(),
            UnitActiveState::UnitActivating
        );
        assert_eq!(
            SocketState::StartChown.to_unit_active_state(),
            UnitActiveState::UnitActivating
        );
        assert_eq!(
            SocketState::StartPost.to_unit_active_state(),
            UnitActiveState::UnitActivating
        );
        assert_eq!(
            SocketState::Listening.to_unit_active_state(),
            UnitActiveState::UnitActive
        );
        assert_eq!(
            SocketState::Running.to_unit_active_state(),
            UnitActiveState::UnitActive
        );
        assert_eq!(
            SocketState::StopPre.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::StopPreSigterm.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::StopPost.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::StopPreSigkill.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::FinalSigterm.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::FinalSigterm.to_unit_active_state(),
            UnitActiveState::UnitDeActivating
        );
        assert_eq!(
            SocketState::Failed.to_unit_active_state(),
            UnitActiveState::UnitFailed
        );
        assert_eq!(
            SocketState::Cleaning.to_unit_active_state(),
            UnitActiveState::UnitMaintenance
        );
    }
}