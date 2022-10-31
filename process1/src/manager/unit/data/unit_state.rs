use bitflags::bitflags;

/**Unit stats：
 A[UnitActive]
 B[UnitReloading]
 C[UnitInActive]
 D[UnitFailed]
 E[UnitActivating]
 F[UnitDeActivating]
 G[UnitMaintenance]
 ```graph LR
C[UnitInActive] -> E[UnitActivating]
E->A[UnitActive]
B[UnitReloading] -> E
E->F[UnitDeActivating]
E->D[UnitFailed]
```
*/

///
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum UnitActiveState {
    ///
    UnitActive,
    ///
    UnitReloading,
    ///
    UnitInActive,
    ///
    UnitFailed,
    ///
    UnitActivating,
    ///
    UnitDeActivating,
    ///
    UnitMaintenance,
}

bitflags! {
    ///
    pub struct UnitNotifyFlags: u8 {
        ///
        const UNIT_NOTIFY_RELOAD_FAILURE = 1 << 0;
        ///
        const UNIT_NOTIFY_WILL_AUTO_RESTART = 1 << 1;
    }
}

#[derive(Debug)]
pub(in crate::manager) struct UnitState {
    pub(in crate::manager) os: UnitActiveState,
    pub(in crate::manager) ns: UnitActiveState,
    pub(in crate::manager) flags: UnitNotifyFlags,
}

impl UnitState {
    pub(in crate::manager) fn new(
        os: UnitActiveState,
        ns: UnitActiveState,
        flags: UnitNotifyFlags,
    ) -> UnitState {
        UnitState { os, ns, flags }
    }
}
