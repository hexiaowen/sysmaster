#![allow(non_snake_case)]
use confique::{Config, Error};

use crate::manager::unit::uload_util::UnitFile;
use crate::manager::unit::unit_base::JobMode;
use crate::manager::unit::DeserializeWith;

#[derive(Config, Default)]
pub(crate) struct UeConfig {
    #[config(nested)]
    pub Unit: UeConfigUnit,
    #[config(nested)]
    pub Install: UeConfigInstall,
}

#[derive(Config, Default)]
pub(crate) struct UeConfigUnit {
    #[config(default = "")]
    pub Description: String,
    #[config(default = "")]
    pub Documentation: String,
    #[config(default = false)]
    pub AllowIsolate: bool,
    #[config(default = false)]
    pub IgnoreOnIsolate: bool,
    // #[config(deserialize_with = JobMode::deserialize_with)]
    // #[config(default = "replace")]
    // pub on_success_job_mode: JobMode,
    #[config(deserialize_with = JobMode::deserialize_with)]
    #[config(default = "replace")]
    pub OnFailureJobMode: JobMode,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub Wants: Vec<String>,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub Requires: Vec<String>,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub Before: Vec<String>,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub After: Vec<String>,
}

#[derive(Config, Default)]
pub(crate) struct UeConfigInstall {
    #[config(default = "")]
    pub Alias: String,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub WantedBy: Vec<String>,
    #[config(deserialize_with = Vec::<String>::deserialize_with)]
    #[config(default = "")]
    pub RequiredBy: Vec<String>,
    #[config(default = "")]
    pub Also: String,
    #[config(default = "")]
    pub DefaultInstance: String,
    // #[config(default = "")]
    // pub install_alias: String,
    // #[config(default = "")]
    // pub install_also: String,
    // #[config(default = "")]
    // pub install_default_install: String,
}

impl UeConfig {
    pub fn load_fragment_and_dropin(
        &self,
        files: &UnitFile,
        name: &String,
    ) -> Result<UeConfig, Error> {
        let mut builder = UeConfig::builder().env();

        // fragment
        for v in files.get_unit_id_fragment_pathbuf(name) {
            builder = builder.file(&v);
        }

        let mut configer = builder.load()?;

        // dropin
        for v in files.get_unit_id_dropin_wants(name) {
            configer.Unit.Wants.push(v.to_string_lossy().to_string());
            configer.Unit.After.push(v.to_string_lossy().to_string());
        }

        for v in files.get_unit_id_dropin_requires(name) {
            configer.Unit.Requires.push(v.to_string_lossy().to_string());
            configer.Unit.After.push(v.to_string_lossy().to_string());
        }
        Ok(configer)
    }
}
