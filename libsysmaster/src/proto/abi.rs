/// Command request from client
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CommandRequest {
    #[prost(oneof="command_request::RequestData", tags="1, 2, 3, 4, 5")]
    pub request_data: ::core::option::Option<command_request::RequestData>,
}
/// Nested message and enum types in `CommandRequest`.
pub mod command_request {
    #[rustfmt::skip]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum RequestData {
        ///unit lifecycle
        #[prost(message, tag="1")]
        Ucomm(super::UnitComm),
        ///unit file commands
        #[prost(message, tag="2")]
        Ufile(super::UnitFile),
        ///job management
        #[prost(message, tag="3")]
        Jcomm(super::JobComm),
        ///manager commands
        #[prost(message, tag="4")]
        Mcomm(super::MngrComm),
        ///system commands, reboot/shutdown/halt
        #[prost(message, tag="5")]
        Syscomm(super::SysComm),
    }
}
/// Command Response from server
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CommandResponse {
    /// stat code; HTTP 2xx/4xx/5xx
    #[prost(uint32, tag="1")]
    pub status: u32,
    /// if not 2xx，message include more information
    #[prost(string, tag="2")]
    pub message: ::prost::alloc::string::String,
}
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnitComm {
    #[prost(enumeration="unit_comm::Action", tag="1")]
    pub action: i32,
    #[prost(string, tag="2")]
    pub unitname: ::prost::alloc::string::String,
}
/// Nested message and enum types in `UnitComm`.
pub mod unit_comm {
    #[rustfmt::skip]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Action {
        Status = 0,
        Start = 1,
        Stop = 2,
        Restart = 3,
        Reload = 4,
        Kill = 5,
    }
}
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnitFile {
    #[prost(enumeration="unit_file::Action", tag="1")]
    pub action: i32,
    #[prost(string, tag="2")]
    pub unitname: ::prost::alloc::string::String,
}
/// Nested message and enum types in `UnitFile`.
pub mod unit_file {
    #[rustfmt::skip]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Action {
        Cat = 0,
        Enable = 1,
        Disable = 2,
        Mask = 3,
        Getdef = 4,
        Setdef = 5,
    }
}
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct JobComm {
    #[prost(enumeration="job_comm::Action", tag="1")]
    pub action: i32,
    #[prost(string, tag="2")]
    pub job_id: ::prost::alloc::string::String,
}
/// Nested message and enum types in `JobComm`.
pub mod job_comm {
    #[rustfmt::skip]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Action {
        List = 0,
        Cancel = 1,
    }
}
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MngrComm {
    #[prost(enumeration="mngr_comm::Action", tag="1")]
    pub action: i32,
}
/// Nested message and enum types in `MngrComm`.
pub mod mngr_comm {
    #[rustfmt::skip]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Action {
        Reload = 0,
        Reexec = 1,
    }
}
#[rustfmt::skip]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SysComm {
    #[prost(enumeration="sys_comm::Action", tag="1")]
    pub action: i32,
}
/// Nested message and enum types in `SysComm`.
pub mod sys_comm {
    #[rustfmt::skip]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Action {
        Reboot = 0,
        Shutdown = 1,
        Halt = 2,
        Suspend = 3,
        Poweroff = 4,
        Hibernate = 5,
    }
}
