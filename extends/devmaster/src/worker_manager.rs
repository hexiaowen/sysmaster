//! worker manager
//!
use crate::job_queue::{DeviceJob, JobState};
use crate::utils::{log_debug, log_info, Error};
use crate::{log_error, JobQueue};
use libdevice::Device;
use libevent::{EventState, EventType, Events, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{self, Display};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::DerefMut;
use std::os::unix::prelude::{AsRawFd, RawFd};
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::thread::JoinHandle;

/// worker manager listen address
pub const WORKER_MANAGER_LISTEN_ADDR: &str = "0.0.0.0:1223";
/// max time interval for idle worker
const WORKER_MAX_IDLE_INTERVAL: u64 = 1;

/// messages sended by manager to workers
pub(crate) enum WorkerMessage {
    Job(Box<Device>),
    Cmd(String),
}

/// worker manager
#[derive(Debug)]
pub struct WorkerManager {
    // events: Rc<libevent::Events>,
    workers_capacity: u32,
    workers: RefCell<HashMap<u32, Rc<Worker>>>,
    listen_addr: String,
    listener: RefCell<TcpListener>,

    kill_idle_workers: RefCell<Option<Rc<WorkerManagerKillWorkers>>>,

    job_queue: RefCell<Weak<JobQueue>>,
    events: Rc<Events>,
}

/// worker
#[derive(Debug)]
pub struct Worker {
    id: u32,
    tx: mpsc::Sender<WorkerMessage>,
    state: RefCell<WorkerState>,
    handler: RefCell<Option<JoinHandle<()>>>,

    device_job: RefCell<Option<Weak<DeviceJob>>>,
}

/// state of worker
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum WorkerState {
    Undef,
    Idle,
    Running,
    Killing, // no longer dispatch device job to this worker, waiting for its ack
    _Killed, // this worker is dead, waiting to recycle it from worker manager
}

impl Display for WorkerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self {
            WorkerState::Undef => "Undef",
            WorkerState::Idle => "Idle",
            WorkerState::Running => "Running",
            WorkerState::Killing => "Killing",
            WorkerState::_Killed => "Killed",
        };

        write!(f, "{state}")
    }
}

impl Worker {
    fn new(id: u32, state: WorkerState, tcp_address: String) -> Worker {
        let (tx, rx) = mpsc::channel::<WorkerMessage>();

        let handler = std::thread::spawn(move || loop {
            let msg = rx.recv().unwrap_or_else(|error| {
                log_error(format!("Worker {id}: panic at recv \"{error}\"\n"));
                panic!();
            });

            match msg {
                WorkerMessage::Job(device) => {
                    log_info(format!(
                        "Worker {id}: received device \"{}\"\n",
                        device.devname
                    ));

                    Self::worker_process_device(id, *device);

                    log_info(format!("Worker {id}: finished job\n"));

                    let mut tcp_stream =
                        TcpStream::connect(tcp_address.as_str()).unwrap_or_else(|error| {
                            log_error(format!("Worker {id}: failed to connect {error}\n"));
                            panic!();
                        });

                    tcp_stream
                        .write_all(format!("finished {id}").as_bytes())
                        .unwrap_or_else(|error| {
                            log_error(format!(
                                "Worker {id}: failed to send ack to manager \"{error}\"\n"
                            ));
                        });
                }
                WorkerMessage::Cmd(cmd) => {
                    log_info(format!("Worker {id} received cmd: {cmd}\n"));
                    match cmd.as_str() {
                        "kill" => {
                            let mut tcp_stream = TcpStream::connect(tcp_address.as_str())
                                .unwrap_or_else(|error| {
                                    log_error(format!(
                                        "Worker {id}: failed to connect \"{error}\"\n"
                                    ));
                                    panic!();
                                });
                            let _ret = tcp_stream
                                .write(format!("killed {id}").as_bytes())
                                .unwrap_or_else(|error| {
                                    log_error(format!(
                                        "Worker {id}: failed to send killed message to manager \"{error}\"\n"
                                    ));
                                    0
                                });
                            log_debug(format!("Worker {id}: is killed\n"));
                            break;
                        }
                        _ => {
                            todo!();
                        }
                    }
                }
            }
        });

        Worker {
            id,
            tx,
            state: RefCell::new(state),
            handler: RefCell::new(Some(handler)),
            device_job: RefCell::new(None),
        }
    }

    /// get the id of the worker
    pub(crate) fn get_id(&self) -> u32 {
        self.id
    }

    /// get the state of the worker
    pub(crate) fn _get_state(&self) -> WorkerState {
        *self.state.borrow()
    }

    /// process a device
    fn worker_process_device(id: u32, device: Device) {
        // log_info(format!("Worker {}: Processing: {:?}\n", id, device));
        log_info(format!("Worker {id}: Processing: {}\n", device.devpath));
        // std::thread::sleep(std::time::Duration::from_secs(5));
    }

    /// send message to the worker thread
    fn worker_send_message(&self, msg: WorkerMessage) {
        self.tx.send(msg).unwrap_or_else(|error| {
            log_error(format!(
                "Worker Manager: failed to send message to worker {}, {error}\n",
                self.id
            ))
        });
    }

    /// bind a worker to a device job
    pub(crate) fn bind(self: &Rc<Worker>, job: &Rc<DeviceJob>) {
        *self.device_job.borrow_mut() = Some(Rc::downgrade(job));
    }

    /// free the device job
    pub(crate) fn job_free(self: &Rc<Worker>) {
        *self.device_job.borrow_mut() = None;
    }
}

impl WorkerManager {
    ///
    pub fn new(workers_capacity: u32, listen_addr: String, events: Rc<Events>) -> WorkerManager {
        WorkerManager {
            workers_capacity,
            workers: RefCell::new(HashMap::new()),
            listen_addr: listen_addr.clone(),
            listener: RefCell::new(TcpListener::bind(listen_addr.as_str()).unwrap_or_else(
                |error| {
                    log_error(format!(
                        "Worker Manager: failed to bind listener \"{error}\"\n"
                    ));
                    panic!();
                },
            )),
            kill_idle_workers: RefCell::new(None),
            job_queue: RefCell::new(Weak::new()),
            events,
        }
    }

    /// set the libevent source instance of kill workers timer
    pub fn set_kill_workers_timer(self: &Rc<WorkerManager>) {
        *self.kill_idle_workers.borrow_mut() = Some(Rc::new(WorkerManagerKillWorkers::new(
            WORKER_MAX_IDLE_INTERVAL,
            self.clone(),
        )));
    }

    /// get the libevent source instance of kill workers timer
    pub fn get_kill_workers_timer(
        self: &Rc<WorkerManager>,
    ) -> Option<Rc<WorkerManagerKillWorkers>> {
        if let Some(source) = self.kill_idle_workers.borrow().as_ref() {
            return Some(source.clone());
        }

        None
    }

    /// set the reference to a job queue instance
    pub fn set_job_queue(&self, job_queue: &Rc<JobQueue>) {
        *self.job_queue.borrow_mut() = Rc::downgrade(job_queue);
    }

    /// create a new worker
    pub(crate) fn create_new_worker(self: &Rc<WorkerManager>) -> Option<u32> {
        for id in 0..self.workers_capacity {
            if !self.workers.borrow().contains_key(&id) {
                self.workers.borrow_mut().insert(
                    id,
                    Rc::new(Worker::new(
                        id,
                        WorkerState::Undef,
                        self.listen_addr.clone(),
                    )),
                );
                log_debug(format!("Worker Manager: created new worker {id}\n"));
                self.set_worker_state(id, WorkerState::Idle);
                return Some(id);
            }
        }

        None
    }

    /// dispatch job to a worker
    pub fn job_dispatch(
        self: &Rc<WorkerManager>,
        device_job: Rc<DeviceJob>,
    ) -> Result<Rc<Worker>, Error> {
        log_debug(format!(
            "Worker Manager: start dispatch job {}\n",
            device_job.seqnum
        ));

        if *device_job.state.borrow() == JobState::Running {
            log_debug(format!(
                "Worker Manager: skip job {} as it is running\n",
                device_job.seqnum
            ));
        }

        for (id, worker) in self.workers.borrow().iter() {
            let state = *worker.state.borrow();
            if state == WorkerState::Idle {
                log_debug(format!("Worker Manager: find idle worker {}\n", worker.id));
                self.set_worker_state(*id, WorkerState::Running);
                worker.worker_send_message(WorkerMessage::Job(Box::new(device_job.device.clone())));
                return Ok(worker.clone());
            }
        }

        if (self.workers.borrow().len() as u32) < self.workers_capacity {
            if let Some(id) = self.create_new_worker() {
                let workers = self.workers.borrow();
                let worker = workers.get(&id).unwrap();
                self.set_worker_state(id, WorkerState::Running);
                worker.worker_send_message(WorkerMessage::Job(Box::new(device_job.device.clone())));
                return Ok(worker.clone());
            }
        }

        Err(Error::WorkerManagerError {
            msg: "failed to get an idle worker for job\n",
        })
    }

    /// update the state of worker according to the ack
    pub fn worker_response_dispose(&self, ack: String) {
        let tokens: Vec<&str> = ack.split(' ').collect();

        if tokens.len() != 2 {
            return;
        }

        let (ack_kind, id) = (
            tokens[0],
            tokens[1]
                .parse::<u32>()
                .expect("Worker respond with invalid id"),
        );

        match ack_kind {
            "killed" => {
                // cleanup the killed worker from the manager
                log_debug(format!("Worker Manager: cleanup worker {id}\n"));

                self.workers
                    .borrow_mut()
                    .deref_mut()
                    .remove(&id)
                    .unwrap()
                    .handler
                    .take()
                    .unwrap()
                    .join()
                    .unwrap();
            }
            "finished" => {
                // log_debug(format!("Worker Manager: set Idle on worker {}\n", id));

                let job = &self
                    .workers
                    .borrow()
                    .get(&id)
                    .unwrap()
                    .device_job
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .unwrap();

                self.set_worker_state(id, WorkerState::Idle);
                self.job_queue.borrow().upgrade().unwrap().job_free(job);

                self.job_queue.borrow().upgrade().unwrap().job_queue_start();
            }
            _ => {
                todo!();
            }
        }
    }

    /// set the state of the worker
    fn set_worker_state(&self, id: u32, state: WorkerState) {
        log_debug(format!("Worker Manager: set Idle on worker {id}\n"));
        let workers = self.workers.borrow();
        let worker = workers.get(&id).unwrap();

        *worker.state.borrow_mut() = state;
    }

    /// kill all workers
    fn manager_kill_workers(&self) {
        for (id, worker) in self.workers.borrow().iter() {
            self.set_worker_state(*id, WorkerState::Killing);
            worker.worker_send_message(WorkerMessage::Cmd(String::from("kill")));
        }
    }

    /// start kill workers timer
    pub fn start_kill_workers_timer(self: &Rc<WorkerManager>) {
        self.events
            .set_enabled(self.get_kill_workers_timer().unwrap(), EventState::Off)
            .unwrap();
        self.events
            .set_enabled(self.get_kill_workers_timer().unwrap(), EventState::OneShot)
            .unwrap();
    }

    /// stop kill workers timer
    pub fn stop_kill_workers_timer(self: &Rc<WorkerManager>) {
        self.events
            .set_enabled(self.get_kill_workers_timer().unwrap(), EventState::Off)
            .unwrap();
    }
}

impl Source for WorkerManager {
    fn fd(&self) -> RawFd {
        self.listener.borrow().as_raw_fd()
    }

    fn event_type(&self) -> libevent::EventType {
        libevent::EventType::Io
    }

    fn epoll_event(&self) -> u32 {
        (libc::EPOLLIN) as u32
    }

    /// Set the priority, -127i8 ~ 128i8, the smaller the value, the higher the priority
    fn priority(&self) -> i8 {
        10
    }

    /// start dispatching after the event arrives
    fn dispatch(&self, _: &libevent::Events) -> Result<i32, libevent::Error> {
        let (mut stream, _) = self.listener.borrow_mut().accept().unwrap();
        let mut ack = String::new();
        stream.read_to_string(&mut ack).unwrap();

        log_debug(format!("Worker Manager: received message \"{ack}\"\n"));
        self.worker_response_dispose(ack);

        Ok(0)
    }

    /// Unless you can guarantee all types of token allocation, it is recommended to use the default implementation here
    fn token(&self) -> u64 {
        let data: u64 = unsafe { std::mem::transmute(self) };
        data
    }
}

/// libevent source to kill workers
#[derive(Debug)]
pub struct WorkerManagerKillWorkers {
    /// time interval
    time: u64,

    /// reference to worker manager
    worker_manager: Weak<WorkerManager>,
}

impl WorkerManagerKillWorkers {
    ///
    fn new(time: u64, worker_manager: Rc<WorkerManager>) -> WorkerManagerKillWorkers {
        WorkerManagerKillWorkers {
            time,
            worker_manager: Rc::downgrade(&worker_manager),
        }
    }
}

impl Source for WorkerManagerKillWorkers {
    ///
    fn fd(&self) -> RawFd {
        0
    }

    ///
    fn event_type(&self) -> EventType {
        EventType::TimerMonotonic
    }

    ///
    fn epoll_event(&self) -> u32 {
        (libc::EPOLLIN) as u32
    }

    ///
    fn priority(&self) -> i8 {
        // -50
        -55
    }

    ///
    fn time_relative(&self) -> u64 {
        self.time * 1000000
    }

    ///
    fn dispatch(&self, _: &Events) -> Result<i32, libevent::Error> {
        log_info("Worker Manager Kill Workers timeout!\n".to_string());
        self.worker_manager
            .upgrade()
            .unwrap()
            .manager_kill_workers();
        Ok(0)
    }

    ///
    fn token(&self) -> u64 {
        let data: u64 = unsafe { std::mem::transmute(self) };
        data
    }
}