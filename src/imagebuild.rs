use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::prelude::OsStrExt,
    path::Path,
};

use indexmap::IndexSet;
use pyxis_parcel::{InodeKind, ParcelHandle, ReaderWriter};
use sys_mount::Unmount;

use crate::{
    chroot::run_in_chroot, get_deps, get_parcel_path, get_provider, hookfile, pyxis_parcel_build,
    ParcelProvider,
};

pub fn get_image_packages(manifest: &str) -> IndexSet<(ParcelProvider, String)> {
    let f = File::open(manifest).unwrap();
    let br = BufReader::new(f);

    let mut to_install = IndexSet::new();
    let mut dep_stack = Vec::new();
    let mut visited = HashSet::new();

    for line in br.lines() {
        let l = line.unwrap();
        if l.starts_with('#') {
            continue;
        }
        let (provider, package) = get_provider(&l);
        dep_stack.push((provider, package));
        while let Some(package) = dep_stack.pop() {
            if to_install.contains(&package) {
                continue;
            }
            let mut to_push = Vec::new();
            for dep in get_deps(package.0, package.1.clone()) {
                if !to_install.contains(&dep) {
                    to_push.push(dep)
                }
            }
            if to_push.is_empty() {
                to_install.insert(package);
            } else if visited.contains(&package) {
                to_install.insert(package);
                dep_stack.append(&mut to_push);
            } else {
                dep_stack.push(package.clone());
                dep_stack.append(&mut to_push);
                visited.insert(package.clone());
            }
        }
    }

    for (provider, package) in &to_install {
        pyxis_parcel_build(*provider, package);
    }
    to_install
}

pub fn pyxis_image_build(manifest: &str) {
    let to_install = get_image_packages(manifest);

    let sty = indicatif::ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {wide_bar} {pos:>5}/{len:5} {msg:>25}")
        .progress_chars("##-");
    let mount_result = sys_mount::Mount::new(
        "tmpfs",
        "temp",
        "tmpfs",
        sys_mount::MountFlags::empty(),
        Some("size=5G"),
    );

    let mount = mount_result
        .unwrap()
        .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    println!("Extracting packages");
    let pb = indicatif::ProgressBar::new(to_install.len() as u64);
    pb.set_style(sty.clone());
    for (provider, package) in &to_install {
        pb.set_message(package.clone());
        pb.tick();
        let f = File::open(get_parcel_path(*provider, package))
            .unwrap_or_else(|_| panic!("Could not find parcel {}", package));
        let reader = Box::new(ReaderWriter::new(f));
        let mut parcel = ParcelHandle::load(reader).unwrap();
        let mut f = File::open(get_parcel_path(*provider, package)).unwrap();
        extract_parcel(&mut f, &mut parcel, 1, "temp");
        pb.inc(1);
    }
    pb.finish();

    let proc_mount = sys_mount::Mount::new(
        "proc",
        "temp/proc",
        "proc",
        sys_mount::MountFlags::NOSUID
            | sys_mount::MountFlags::NOEXEC
            | sys_mount::MountFlags::NODEV,
        None,
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    let sys_mount = sys_mount::Mount::new(
        "sys",
        "temp/sys",
        "sysfs",
        sys_mount::MountFlags::NOSUID
            | sys_mount::MountFlags::NOEXEC
            | sys_mount::MountFlags::NODEV
            | sys_mount::MountFlags::RDONLY,
        None,
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    let dev_mount = sys_mount::Mount::new(
        "udev",
        "temp/dev",
        "devtmpfs",
        sys_mount::MountFlags::NOSUID,
        Some("mode=0755"),
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    let devpts_mount = sys_mount::Mount::new(
        "devpts",
        "temp/dev/pts",
        "devpts",
        sys_mount::MountFlags::NOSUID | sys_mount::MountFlags::NOEXEC,
        Some("mode=0620,gid=5"),
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    let devshm_mount = sys_mount::Mount::new(
        "shm",
        "temp/dev/shm",
        "tmpfs",
        sys_mount::MountFlags::NOSUID | sys_mount::MountFlags::NODEV,
        Some("mode=1777"),
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    let tmp_mount = sys_mount::Mount::new(
        "tmp",
        "temp/tmp",
        "tmpfs",
        sys_mount::MountFlags::NOSUID
            | sys_mount::MountFlags::NODEV
            | sys_mount::MountFlags::STRICTATIME,
        Some("mode=1777"),
    )
    .unwrap()
    .into_unmount_drop(sys_mount::UnmountFlags::DETACH);

    println!("Running actions");
    let pb = indicatif::ProgressBar::new(to_install.len() as u64);
    pb.set_style(sty);
    for (provider, package) in &to_install {
        pb.set_message(package.clone());
        pb.tick();

        if !Path::new(&format!(
            "temp/.PYXIS/{}/{}/.INSTALL",
            provider.as_str(),
            package
        ))
        .exists()
        {
            continue;
        } else {
            pb.println(format!("Found scriptlets for {:?}|{}", provider, package));
        }

        let cmdline = format!(". /.PYXIS/{}/{}/.INSTALL; declare -F post_install && post_install {} || echo No install action",provider.as_str(),package,"0");

        run_in_chroot(cmdline, "".to_string());
        pb.inc(1);
    }
    pb.finish();
    println!("Running hooks");

    let mut hooks = if let Ok(h) = std::fs::read_dir("temp/usr/share/libalpm/hooks/") {
        h.map(|res| res.map(|e| e.path()))
            .collect::<Result<Vec<_>, std::io::Error>>()
            .unwrap()
    } else {
        println!("No hooks directory");
        Vec::new()
    };
    hooks.sort();
    for hook in hooks {
        let hook = hookfile::parse_hook(&mut File::open(hook).unwrap());
        println!("{:?}", hook.action.description);
        let mut triggers = Vec::new();
        for trigger in hook.triggers {
            if trigger
                .operations
                .contains(&hookfile::HookTriggerOperation::Install)
            {
                if trigger.flavor == hookfile::HookTriggerFlavor::Package {
                    for pkg in trigger.targets {
                        if to_install.iter().any(|i| i.1 == pkg) {
                            println!("Package hook {} triggered", pkg);
                            triggers.push(pkg);
                        }
                    }
                } else {
                    'hookloop: for path in trigger.targets {
                        for res in glob::glob(&("temp/".to_owned() + &path)).unwrap() {
                            let res = res.unwrap().into_os_string().into_string().unwrap()[4..]
                                .to_owned();
                            println!("Path hook '{}' triggered on '{}'", path, res);
                            triggers.push(res);
                            if !hook.action.needs_targets {
                                println!("Stopping check, do not need full target list");
                                break 'hookloop;
                            }
                        }
                    }
                }
            }
        }
        if !triggers.is_empty() {
            assert_eq!(hook.action.when, hookfile::HookActionWhen::PostTransaction);
            assert!(hook.action.depends.is_empty());
            assert!(!hook.action.abort_on_fail);
            if hook.action.needs_targets {
                run_in_chroot(hook.action.exec, triggers.join("\n"));
            } else {
                run_in_chroot(hook.action.exec, "".to_string());
            }
        }
    }

    std::mem::drop(proc_mount);
    std::mem::drop(sys_mount);
    std::mem::drop(dev_mount);
    std::mem::drop(devpts_mount);
    std::mem::drop(devshm_mount);
    std::mem::drop(tmp_mount);

    std::process::Command::new("rsync")
        .args(["-ah", "--delete", "temp/", "/tmp/build-pyxis/"])
        .status()
        .expect("failed to execute process");

    std::mem::drop(mount);
}

fn extract_parcel(pf: &mut File, parcel: &mut ParcelHandle, ino: u64, ex_dir: &str) {
    if std::fs::metadata(ex_dir).is_err() {
        std::fs::create_dir(ex_dir).unwrap();
    }
    let attr = parcel.getattr(ino).unwrap();
    std::fs::set_permissions(
        ex_dir,
        std::os::unix::fs::PermissionsExt::from_mode(attr.perm.into()),
    )
    .unwrap();
    nix::unistd::chown(
        ex_dir,
        Some(nix::unistd::Uid::from_raw(attr.uid)),
        Some(nix::unistd::Gid::from_raw(attr.gid)),
    )
    .unwrap();
    for (ino, kind, name) in parcel.readdir(ino).unwrap() {
        match kind {
            InodeKind::Directory => {
                extract_parcel(pf, parcel, ino, &(String::from(ex_dir) + "/" + &name));
            }
            InodeKind::RegularFile => {
                let fnm = String::from(ex_dir) + "/" + &name;
                let mut f = File::create(fnm.clone()).unwrap();
                f.write_all(&parcel.read(ino, 0, None).unwrap()).unwrap();
                let attr = parcel.getattr(ino).unwrap();
                nix::unistd::chown(
                    fnm.as_str(),
                    Some(nix::unistd::Uid::from_raw(attr.uid)),
                    Some(nix::unistd::Gid::from_raw(attr.gid)),
                )
                .unwrap();
                std::fs::set_permissions(
                    fnm.clone(),
                    std::os::unix::fs::PermissionsExt::from_mode(attr.perm.into()),
                )
                .unwrap();
            }
            InodeKind::Symlink => {
                let fnm = String::from(ex_dir) + "/" + &name;
                std::os::unix::fs::symlink(
                    Path::new(std::ffi::OsStr::from_bytes(&parcel.readlink(ino).unwrap())),
                    fnm.clone(),
                )
                .unwrap();
            }
            InodeKind::CharDevice => {
                unimplemented!();
            }
            InodeKind::Whiteout => {
                unimplemented!();
            }
        }
    }
}
