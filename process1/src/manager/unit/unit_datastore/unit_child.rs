use super::unit_sets::UnitSets;
use crate::manager::table::{TableOp, TableSubscribe};
use crate::manager::unit::unit_entry::UnitX;
use crate::manager::unit::unit_rentry::UnitRe;
use crate::reliability::ReStation;
use nix::unistd::Pid;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub(super) struct UnitChild {
    // associated objects
    units: Rc<UnitSets>,

    // owned objects
    sub_name: String, // key for table-subscriber: UnitSets
    data: Rc<UnitChildData>,
}

impl ReStation for UnitChild {
    // no input, no compensate

    // data
    fn db_map(&self) {
        self.data.db_map(&self.units);
    }

    // reload
    fn entry_clear(&self) {
        self.data.entry_clear();
    }
}

impl UnitChild {
    pub(super) fn new(rentryr: &Rc<UnitRe>, unitsr: &Rc<UnitSets>) -> UnitChild {
        let uc = UnitChild {
            units: Rc::clone(unitsr),
            sub_name: String::from("UnitChild"),
            data: Rc::new(UnitChildData::new(rentryr)),
        };
        uc.register();
        uc
    }

    pub(super) fn add_watch_pid(&self, id: &str, pid: Pid) {
        log::debug!("borrow add watch_pids for pid:{}, id:{}", pid, id);
        let unit = self.units.get(id).unwrap();
        let u = Rc::clone(&unit);
        self.data.add_watch_pid(unit, pid);
        u.child_add_pids(pid);
    }

    pub(super) fn unwatch_pid(&self, id: &str, pid: Pid) {
        let unit = self.units.get(id).unwrap();
        let u = Rc::clone(&unit);
        log::debug!("borrow remove watch_pids for {}", pid);
        self.data.unwatch_pid(unit, pid);
        u.child_remove_pids(pid);
    }

    pub(super) fn get_unit_by_pid(&self, pid: Pid) -> Option<Rc<UnitX>> {
        self.data.get_unit_by_pid(pid)
    }

    fn register(&self) {
        // db-units
        let subscriber = Rc::clone(&self.data);
        self.units.register(&self.sub_name, subscriber);
    }
}

struct UnitChildData {
    // associated objects
    rentry: Rc<UnitRe>,

    // owned objects
    watch_pids: RefCell<HashMap<Pid, Rc<UnitX>>>, // key: pid, value: units
}

impl TableSubscribe<String, Rc<UnitX>> for UnitChildData {
    fn notify(&self, op: &TableOp<String, Rc<UnitX>>) {
        match op {
            TableOp::TableInsert(_, _) => {} // do nothing
            TableOp::TableRemove(_, unit) => self.remove_unit(unit),
        }
    }
}

// the declaration "pub(self)" is for identification only.
impl UnitChildData {
    pub(self) fn new(rentryr: &Rc<UnitRe>) -> UnitChildData {
        UnitChildData {
            rentry: Rc::clone(rentryr),
            watch_pids: RefCell::new(HashMap::new()),
        }
    }

    pub(self) fn entry_clear(&self) {
        self.watch_pids.borrow_mut().clear();
    }

    pub(self) fn db_map(&self, units: &UnitSets) {
        for unit_id in self.rentry.child_keys().iter() {
            let unit = units.get(unit_id).unwrap();
            for pid in self.rentry.child_get(unit_id).iter() {
                self.add_watch_pid(Rc::clone(&unit), *pid);
            }
        }
    }

    pub(self) fn add_watch_pid(&self, unit: Rc<UnitX>, pid: Pid) {
        let mut watch_pids = self.watch_pids.borrow_mut();
        watch_pids.insert(pid, unit);
    }

    pub(self) fn unwatch_pid(&self, _unit: Rc<UnitX>, pid: Pid) {
        self.watch_pids.borrow_mut().remove(&pid);
    }

    pub(self) fn get_unit_by_pid(&self, pid: Pid) -> Option<Rc<UnitX>> {
        self.watch_pids.borrow().get(&pid).cloned()
    }

    fn remove_unit(&self, _unit: &UnitX) {
        todo!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::rentry::RELI_HISTORY_MAX_DBS;
    use crate::manager::unit::data::DataManager;
    use crate::manager::unit::uload_util::UnitFile;
    use crate::manager::unit::unit_rentry::{UnitRe, UnitType};
    use crate::plugin::Plugin;
    use crate::reliability::Reliability;
    use libutils::logger;

    #[test]
    #[should_panic]
    fn child_add_watch_pid_empty() {
        let reli = Rc::new(Reliability::new(RELI_HISTORY_MAX_DBS));
        let rentry = Rc::new(UnitRe::new(&reli));
        let sets = UnitSets::new();
        let name_test3 = String::from("test3.service");
        let child = UnitChild::new(&rentry, &Rc::new(sets));
        let pid = Pid::from_raw(1);

        child.add_watch_pid(&name_test3, pid);
    }

    #[test]
    fn child_add_watch_pid() {
        let dm = Rc::new(DataManager::new());
        let reli = Rc::new(Reliability::new(RELI_HISTORY_MAX_DBS));
        let rentry = Rc::new(UnitRe::new(&reli));
        let sets = UnitSets::new();
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&dm, &reli, &rentry, &name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&dm, &reli, &rentry, &name_test2);
        sets.insert(name_test1.clone(), Rc::clone(&unit_test1));
        sets.insert(name_test2.clone(), Rc::clone(&unit_test2));
        let child = UnitChild::new(&rentry, &Rc::new(sets));
        let pid1 = Pid::from_raw(1);
        let pid2 = Pid::from_raw(2);

        assert_eq!(child.data.watch_pids.borrow().len(), 0);

        child.add_watch_pid(&name_test1, pid1);
        assert_eq!(child.data.watch_pids.borrow().len(), 1);

        child.add_watch_pid(&name_test2, pid2);
        assert_eq!(child.data.watch_pids.borrow().len(), 2);
    }

    #[test]
    fn child_unwatch_pid() {
        let dm = Rc::new(DataManager::new());
        let reli = Rc::new(Reliability::new(RELI_HISTORY_MAX_DBS));
        let rentry = Rc::new(UnitRe::new(&reli));
        let sets = UnitSets::new();
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&dm, &reli, &rentry, &name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&dm, &reli, &rentry, &name_test2);
        sets.insert(name_test1.clone(), Rc::clone(&unit_test1));
        sets.insert(name_test2.clone(), Rc::clone(&unit_test2));
        let child = UnitChild::new(&rentry, &Rc::new(sets));
        let pid1 = Pid::from_raw(1);
        let pid2 = Pid::from_raw(2);

        assert_eq!(child.data.watch_pids.borrow().len(), 0);

        child.add_watch_pid(&name_test1, pid1);
        child.add_watch_pid(&name_test2, pid2);
        assert_eq!(child.data.watch_pids.borrow().len(), 2);

        child.unwatch_pid(&name_test1, pid1);
        assert_eq!(child.data.watch_pids.borrow().len(), 1);

        child.unwatch_pid(&name_test2, pid2);
        assert_eq!(child.data.watch_pids.borrow().len(), 0);
    }

    fn create_unit(
        dmr: &Rc<DataManager>,
        relir: &Rc<Reliability>,
        rentryr: &Rc<UnitRe>,
        name: &str,
    ) -> Rc<UnitX> {
        logger::init_log_with_console("test_unit_load", 4);
        log::info!("test");
        let file = Rc::new(UnitFile::new());
        let unit_type = UnitType::UnitService;
        let plugins = Plugin::get_instance();
        let subclass = plugins.create_unit_obj(unit_type).unwrap();
        subclass.attach_reli(Rc::clone(relir));
        Rc::new(UnitX::new(
            dmr,
            rentryr,
            &file,
            unit_type,
            name,
            subclass.into_unitobj(),
        ))
    }
}
