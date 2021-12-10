use std::{
    fs::File,
    io::{BufRead, BufReader},
};

#[derive(Eq, PartialEq, Debug)]
pub struct Hook {
    pub triggers: Vec<HookTrigger>,
    pub action:   HookAction,
}

impl Hook {
    pub fn new() -> Hook {
        Hook {
            triggers: Vec::new(),
            action:   HookAction::new(),
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
pub struct HookTrigger {
    pub operations: Vec<HookTriggerOperation>,
    pub flavor:     HookTriggerFlavor,
    pub targets:    Vec<String>,
}

impl HookTrigger {
    pub fn new() -> HookTrigger {
        HookTrigger {
            operations: Vec::new(),
            flavor:     HookTriggerFlavor::None,
            targets:    Vec::new(),
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
pub struct HookAction {
    pub description:   Option<String>,
    pub when:          HookActionWhen,
    pub exec:          String,
    pub depends:       Vec<String>,
    pub abort_on_fail: bool,
    pub needs_targets: bool,
}

impl HookAction {
    pub fn new() -> HookAction {
        HookAction {
            description:   None,
            when:          HookActionWhen::None,
            exec:          String::new(),
            depends:       Vec::new(),
            abort_on_fail: false,
            needs_targets: false,
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
pub enum HookTriggerOperation {
    Install,
    Upgrade,
    Remove,
}

#[derive(Eq, PartialEq, Debug)]
pub enum HookTriggerFlavor {
    Path,
    Package,
    None,
}

#[derive(Eq, PartialEq, Debug)]
pub enum HookActionWhen {
    PreTransaction,
    PostTransaction,
    None,
}

pub fn parse_hook(f: &mut File) -> Hook {
    let mut res = Hook::new();
    let mut section = 0;
    let mut ct = HookTrigger::new();
    let mut ca = HookAction::new();
    for line in BufReader::new(f).lines() {
        let line = line.unwrap().trim().to_string();

        if line.is_empty() {
            continue;
        }

        if line == "[Trigger]" {
            section = 1;
            if ct.flavor != HookTriggerFlavor::None {
                res.triggers.push(ct);
                ct = HookTrigger::new();
            }
            if ca.when != HookActionWhen::None {
                assert_eq!(res.action.when, HookActionWhen::None);
                res.action = ca;
                ca = HookAction::new();
            }
            continue;
        }

        if line == "[Action]" {
            section = 2;
            if ct.flavor != HookTriggerFlavor::None {
                res.triggers.push(ct);
                ct = HookTrigger::new();
            }
            if ca.when != HookActionWhen::None {
                assert_eq!(res.action.when, HookActionWhen::None);
                res.action = ca;
                ca = HookAction::new();
            }
            continue;
        }

        let ls: Vec<&str> = line.split('=').collect();
        if ls.len() == 2 {
            let k = ls[0].trim();
            let v = ls[1].trim();
            match section {
                0 => unimplemented!("Outside of block"),
                1 => match k {
                    "Type" => match v {
                        "Path" => ct.flavor = HookTriggerFlavor::Path,
                        "Package" => ct.flavor = HookTriggerFlavor::Package,
                        _ => unimplemented!("Unknown value for trigger block {}: {}", k, v),
                    },
                    "Operation" => match v {
                        "Install" => ct.operations.push(HookTriggerOperation::Install),
                        "Upgrade" => ct.operations.push(HookTriggerOperation::Upgrade),
                        "Remove" => ct.operations.push(HookTriggerOperation::Remove),
                        _ => unimplemented!("Unknown value for trigger block {}: {}", k, v),
                    },
                    "Target" => {
                        ct.targets.push(v.to_string());
                    }
                    _ => unimplemented!("Unknown key for trigger block: {}", k),
                },
                2 => match k {
                    "Description" => ca.description = Some(v.to_string()),
                    "When" => match v {
                        "PreTransaction" => ca.when = HookActionWhen::PreTransaction,
                        "PostTransaction" => ca.when = HookActionWhen::PostTransaction,
                        _ => unimplemented!("Unknown value for action block {}: {}", k, v),
                    },
                    "Exec" => ca.exec = v.to_string(),
                    _ => unimplemented!("Unknown key for action block: {}", k),
                },
                _ => unimplemented!(),
            }
            continue;
        }
        if ls.len() == 1 {
            let k = ls[0].trim();
            match section {
                0 => unimplemented!("Outside of block"),
                1 => unimplemented!("Unknown key for trigger block: {}", k),
                2 => match k {
                    "NeedsTargets" => ca.needs_targets = true,
                    _ => unimplemented!("Unknown key for action block: {}", k),
                },
                _ => unimplemented!(),
            }
            continue;
        }

        unimplemented!("Unexpected line: {}", line);
    }

    if ct.flavor != HookTriggerFlavor::None {
        res.triggers.push(ct);
    }
    if ca.when != HookActionWhen::None {
        assert_eq!(res.action.when, HookActionWhen::None);
        res.action = ca;
    }

    assert!(!res.triggers.is_empty());
    assert_ne!(res.action.when, HookActionWhen::None);
    res
}
