use lazy_static::lazy_static;
use nix::poll::PollFd;
use nix::poll::PollFlags;
use nix::sys::wait::WaitStatus;
use nix::unistd::execv;
use std::fs::File;
use std::io::BufReader;
use std::io::Write;
use std::os::unix::prelude::OsStrExt;
use std::sync::Mutex;
use sys_mount::Unmount;

use std::path::Path;
use std::path::PathBuf;

use indexmap::set::IndexSet;
use std::collections::HashMap;
use std::collections::HashSet;

use std::io::BufRead;
use std::io::Read;

use nix::fcntl::OFlag;

use pyxis_parcel::Parcel;

mod hookfile;

lazy_static! {
    static ref ALPM_MUTEX: Mutex<Option<alpm::Alpm>> = Mutex::new(None);
}

pub fn with_alpm(f: Box<dyn FnOnce(&alpm::Alpm) -> Vec<String>>) -> Vec<String> {
    let mut mres = ALPM_MUTEX.lock().unwrap();
    if mres.is_none() {
        *mres = Some(alpm::Alpm::new("/", "/var/lib/pacman").unwrap());
        mres.as_ref()
            .unwrap()
            .register_syncdb("core", alpm::SigLevel::USE_DEFAULT)
            .unwrap();
        mres.as_ref()
            .unwrap()
            .register_syncdb("extra", alpm::SigLevel::USE_DEFAULT)
            .unwrap();
        mres.as_ref()
            .unwrap()
            .register_syncdb("community", alpm::SigLevel::USE_DEFAULT)
            .unwrap();
    }
    f(mres.as_ref().unwrap())
}

pub fn alpm_find_satisfier(package: &str) -> Vec<String> {
    let pkb = Box::new(package.to_owned());
    with_alpm(Box::new(|alpm: &alpm::Alpm| {
        let mut res = Vec::new();
        let package = *pkb;
        res.push(
            alpm.syncdbs()
                .find_satisfier(package)
                .unwrap()
                .name()
                .to_owned(),
        );
        res
    }))
}

pub fn alpm_resolve_package(package: &str) -> Vec<String> {
    let pkb = Box::new(package.to_owned());
    with_alpm(Box::new(|alpm: &alpm::Alpm| {
        let mut res = Vec::new();
        let package = *pkb;
        for db in alpm.syncdbs() {
            if let Ok(pkg) = db.pkg(package.clone()) {
                res = vec![db.name().to_owned(), pkg.filename().to_owned()];
            }
        }
        res
    }))
}

pub fn alpm_get_deps(package: &str) -> Vec<String> {
    let pkb = Box::new(package.to_owned());
    with_alpm(Box::new(|alpm: &alpm::Alpm| {
        let mut res = Vec::new();
        let package = *pkb;
        for db in alpm.syncdbs() {
            if let Ok(pkg) = db.pkg(package.clone()) {
                res = pkg.depends().iter().map(|x| x.name().to_owned()).collect();
            }
        }
        res
    }))
}

pub fn alpm_fetch(package: &str) -> (File, String) {
    println!("Fetching {}", package);
    let pkg = alpm_resolve_package(package);
    let repo = &pkg[0];
    let filename = &pkg[1];

    println!("{}", filename);

    let mut easy = curl::easy::Easy::new();
    easy.url(&format!(
        "https://mirrors.xtom.com/archlinux/{}/os/x86_64/{}",
        repo, filename
    ))
    .unwrap();

    let mut file = File::create("foo.tar.zst").unwrap();

    let mut transfer = easy.transfer();
    transfer
        .write_function(|data| {
            file.write_all(data).unwrap();
            Ok(data.len())
        })
        .unwrap();
    transfer.perform().unwrap();
    std::mem::drop(transfer);

    (
        File::open("foo.tar.zst").unwrap(),
        filename.split('.').next_back().unwrap().to_owned(),
    )
}

pub fn get_user() -> String {
    if let Ok(u) = std::env::var("SUDO_USER") {
        u
    } else {
        std::env::var("USER").unwrap()
    }
}

pub fn get_home() -> PathBuf {
    PathBuf::from(passwd::Passwd::from_name(&get_user()).unwrap().home_dir)
}

pub fn get_parcel_path(package: &str) -> PathBuf {
    let mut buf = get_home();
    buf.push(".pyxis/parcel/");
    buf.push(format!("{}.parcel", package));
    buf
}

pub fn exists_parcel(package: &str) -> bool {
    get_parcel_path(package).exists()
}

pub fn parcel_from_pacman<R: Sized + std::io::Read>(package: &str, mut archive: tar::Archive<R>) {
    let mut dir_map: HashMap<PathBuf, u64> = HashMap::new();
    dir_map.insert(PathBuf::from("/"), 1);

    let mut parcel = Parcel::new();

    let time = std::time::SystemTime::now();
    let attr = pyxis_parcel::InodeAttr {
        atime: time,
        ctime: time,
        mtime: time,
        uid: 0,
        gid: 0,
        nlink: 1,
        perm: 0o644,
        rdev: 0,
    };
    let pyxis_dir = parcel.add_directory(attr, HashMap::new());
    let parcel_dir = parcel.add_directory(attr, HashMap::new());
    parcel.insert_dirent(1, std::ffi::OsString::from(".PYXIS"), pyxis_dir);
    parcel.insert_dirent(pyxis_dir, std::ffi::OsString::from(package), parcel_dir);

    for ent in archive.entries().unwrap() {
        let mut e = ent.unwrap();
        let ent_header = e.header().clone();

        let time =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(ent_header.mtime().unwrap());

        let header = ent_header.as_ustar().unwrap();
        let p = e.path().unwrap();
        let p_st = p.to_str().unwrap();
        let entry_path = Path::new("/").join(p_st);
        let parent_inode = *dir_map.get(entry_path.parent().unwrap()).unwrap();
        let entry_name = entry_path.file_name().unwrap();
        match header.typeflag {
            [b'0'] => {
                let attr = pyxis_parcel::InodeAttr {
                    atime: time,
                    ctime: time,
                    mtime: time,
                    uid: ent_header.uid().unwrap() as u32,
                    gid: ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm: ent_header.mode().unwrap(),
                    rdev: 0,
                };
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                let ino = parcel.add_file(pyxis_parcel::FileAdd::Bytes(buf), attr, HashMap::new());
                match (parent_inode, entry_name.to_str().unwrap()) {
                    (1, ".INSTALL" | ".BUILDINFO" | ".MTREE" | ".PKGINFO") => {
                        parcel.insert_dirent(parcel_dir, entry_name.to_owned(), ino)
                    }
                    _ => parcel.insert_dirent(parent_inode, entry_name.to_owned(), ino),
                }
            }
            [b'1'] => {
                let link_name = ent_header
                    .link_name()
                    .unwrap()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned()
                    .into();
                let ino = parcel.add_hardlink(link_name);
                parcel.insert_dirent(parent_inode, entry_name.to_owned(), ino);
            }
            [b'2'] => {
                let link_name = ent_header
                    .link_name()
                    .unwrap()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned()
                    .into();
                let attr = pyxis_parcel::InodeAttr {
                    atime: time,
                    ctime: time,
                    mtime: time,
                    uid: ent_header.uid().unwrap() as u32,
                    gid: ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm: ent_header.mode().unwrap(),
                    rdev: 0,
                };
                let ino = parcel.add_symlink(link_name, attr, HashMap::new());
                parcel.insert_dirent(parent_inode, entry_name.to_owned(), ino);
            }
            [b'5'] => {
                let attr = pyxis_parcel::InodeAttr {
                    atime: time,
                    ctime: time,
                    mtime: time,
                    uid: ent_header.uid().unwrap() as u32,
                    gid: ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm: ent_header.mode().unwrap(),
                    rdev: 0,
                };
                let ino = parcel.add_directory(attr, HashMap::new());
                parcel.insert_dirent(parent_inode, entry_name.to_owned(), ino);
                dir_map.insert(entry_path, ino);
            }
            _ => unimplemented!("TF:{:?}", header.typeflag),
        }
    }
    let parcelpath = get_parcel_path(package);
    std::fs::create_dir_all(parcelpath.parent().unwrap()).unwrap();
    let file = File::create(parcelpath).unwrap();
    parcel.store(file);
}

pub fn pyxis_parcel_fetch(package: &str) {
    let package = &alpm_find_satisfier(package)[0];

    for dep in &alpm_get_deps(package) {
        pyxis_parcel_fetch(dep);
    }

    if exists_parcel(package) {
        return;
    }

    let (f, ext) = alpm_fetch(package);

    match ext.as_str() {
        "zst" => {
            let dec = zstd::stream::read::Decoder::new(f).unwrap();
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(package, archive);
        }
        "xz" => {
            let dec = xz::read::XzDecoder::new(f);
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(package, archive);
        }
        _ => unimplemented!("{}", ext),
    };
}

pub fn pyxis_parcel_local(path: &str) {
    let f = File::open(path).unwrap();
    let ext = path.rsplit('.').next().unwrap();
    let package = path.split('.').next().unwrap();

    match ext {
        "zst" => {
            let dec = zstd::stream::read::Decoder::new(f).unwrap();
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(package, archive);
        }
        "xz" => {
            let dec = xz::read::XzDecoder::new(f);
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(package, archive);
        }
        _ => unimplemented!("{}", ext),
    };
}

pub fn pyxis_image_build(manifest: &str) {
    let f = File::open(manifest).unwrap();
    let br = BufReader::new(f);

    let mut to_install = IndexSet::new();
    let mut dep_stack = Vec::new();
    let mut visited = HashSet::new();

    for line in br.lines() {
        let l = line.unwrap();
        dep_stack.push(l.clone());
        while let Some(package) = dep_stack.pop() {
            if to_install.contains(&package) {
                continue;
            }
            let mut to_push = Vec::new();
            for dep in &alpm_get_deps(&package) {
                if !to_install.contains(dep) {
                    to_push.push(alpm_find_satisfier(dep)[0].clone())
                }
            }
            if to_push.is_empty() {
                to_install.insert(package);
            } else if visited.contains(&package) {
                println!("Circular dependency on {}. Resolving.", package);
                to_install.insert(package);
                dep_stack.append(&mut to_push);
            } else {
                dep_stack.push(package.clone());
                dep_stack.append(&mut to_push);
                visited.insert(package.clone());
            }
        }
    }
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
    for package in &to_install {
        pb.set_message(package.clone());
        pb.tick();
        let f = File::open(get_parcel_path(package))
            .unwrap_or_else(|_| panic!("Could not find parcel {}", package));
        let mut reader = BufReader::new(f);
        let parcel = Parcel::load(&mut reader);
        let mut f = File::open(get_parcel_path(package)).unwrap();
        extract_parcel(&mut f, &parcel, 1, "temp");
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
    for package in &to_install {
        pb.set_message(package.clone());
        pb.tick();

        if !Path::new(&format!("temp/.PYXIS/{}/.INSTALL", package)).exists() {
            continue;
        } else {
            pb.println(format!("Found scriptlets for {}", package));
        }

        let cmdline = format!(". /.PYXIS/{}/.INSTALL; declare -F post_install && post_install {} || echo No install action",package,"0");

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
        println!("{:#?}", hook);
        let mut triggers = Vec::new();
        for trigger in hook.triggers {
            if trigger
                .operations
                .contains(&hookfile::HookTriggerOperation::Install)
            {
                if trigger.flavor == hookfile::HookTriggerFlavor::Package {
                    for pkg in trigger.targets {
                        if to_install.contains(&pkg) {
                            println!("Package hook {} triggered", pkg);
                            triggers.push(pkg);
                        }
                    }
                } else {
                    for path in trigger.targets {
                        for res in glob::glob(&("temp/".to_owned() + &path)).unwrap() {
                            let res = res.unwrap().into_os_string().into_string().unwrap()[4..]
                                .to_owned();
                            println!("Path hook '{}' triggered on '{}'", path, res);
                            triggers.push(res);
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
        .args(["-ah", "--delete", "temp/", "build-pyxis/"])
        .status()
        .expect("failed to execute process");

    std::mem::drop(mount);
}

fn run_in_chroot(cmdline: String, input: String) -> i32 {
    let cwdfd = nix::fcntl::open(
        ".",
        OFlag::O_RDONLY | OFlag::O_CLOEXEC,
        nix::sys::stat::Mode::empty(),
    )
    .unwrap();
    let child2parent_pipefd = nix::sys::socket::socketpair(
        nix::sys::socket::AddressFamily::Unix,
        nix::sys::socket::SockType::Stream,
        None,
        nix::sys::socket::SockFlag::empty(),
    )
    .unwrap();
    let parent2child_pipefd = nix::sys::socket::socketpair(
        nix::sys::socket::AddressFamily::Unix,
        nix::sys::socket::SockType::Stream,
        None,
        nix::sys::socket::SockFlag::empty(),
    )
    .unwrap();
    match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Parent { child, .. }) => {
            nix::unistd::close(child2parent_pipefd.1).unwrap();
            nix::unistd::close(parent2child_pipefd.0).unwrap();
            let mut pollfds = [
                PollFd::new(child2parent_pipefd.0, PollFlags::POLLIN),
                PollFd::new(parent2child_pipefd.1, PollFlags::POLLOUT),
            ];
            let mut buf = [0u8];
            let mut input = input.as_bytes().to_vec();
            loop {
                nix::poll::poll(&mut pollfds, -1).unwrap();
                if let Some(flags0) = pollfds[0].revents() {
                    if let Some(flags1) = pollfds[1].revents() {
                        if flags0.contains(nix::poll::PollFlags::POLLIN) {
                            if nix::unistd::read(child2parent_pipefd.0, &mut buf).unwrap() == 0 {
                                break;
                            }
                            nix::unistd::write(nix::libc::STDOUT_FILENO, &buf).unwrap();
                        } else if flags1.contains(nix::poll::PollFlags::POLLOUT) {
                            if input.is_empty() {
                                println!("Closing");
                                nix::unistd::close(parent2child_pipefd.1).unwrap();
                                pollfds[1] = PollFd::new(-1, PollFlags::empty());
                            } else if nix::unistd::write(parent2child_pipefd.1, &[input.remove(0)])
                                .unwrap()
                                == 0
                            {
                                break;
                            }
                        } else {
                            panic!("Poll error: {:?}", pollfds);
                        }
                    }
                } else {
                    panic!("Poll error: {:?}", pollfds);
                }
            }
            println!();
            let res = nix::sys::wait::waitpid(child, None).unwrap();

            match res {
                WaitStatus::Exited(_, code) => {
                    return code;
                }
                _ => unimplemented!(),
            }
        }
        Ok(nix::unistd::ForkResult::Child) => {
            nix::unistd::close(0).unwrap();
            nix::unistd::close(1).unwrap();
            nix::unistd::close(2).unwrap();
            loop {
                let res = nix::unistd::dup2(parent2child_pipefd.0, 0);
                match res {
                    Ok(_) => break,
                    Err(nix::errno::Errno::EBUSY) => continue,
                    Err(e) => panic!("{}", e),
                }
            }
            loop {
                let res = nix::unistd::dup2(child2parent_pipefd.1, 1);
                match res {
                    Ok(_) => break,
                    Err(nix::errno::Errno::EBUSY) => continue,
                    Err(e) => panic!("{}", e),
                }
            }
            loop {
                let res = nix::unistd::dup2(child2parent_pipefd.1, 2);
                match res {
                    Ok(_) => break,
                    Err(nix::errno::Errno::EBUSY) => continue,
                    Err(e) => panic!("{}", e),
                }
            }
            nix::unistd::close(parent2child_pipefd.0).unwrap();
            nix::unistd::close(parent2child_pipefd.1).unwrap();
            nix::unistd::close(child2parent_pipefd.0).unwrap();
            nix::unistd::close(child2parent_pipefd.1).unwrap();

            nix::unistd::close(cwdfd).unwrap();

            nix::unistd::chroot("temp").unwrap();

            nix::unistd::chdir("/").unwrap();

            execv(
                &std::ffi::CString::new("/bin/bash").unwrap(),
                &["/bin/bash", "-x", "-c", &cmdline].map(|x| std::ffi::CString::new(x).unwrap()),
            )
            .unwrap();

            unsafe { nix::libc::_exit(1) };
        }
        Err(_) => println!("Fork failed"),
    }
    panic!("We should never get here");
}

pub fn extract_parcel(pf: &mut File, parcel: &Parcel, ino: u64, ex_dir: &str) {
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
            pyxis_parcel::InodeKind::Directory => {
                extract_parcel(pf, parcel, ino, &(String::from(ex_dir) + "/" + &name));
            }
            pyxis_parcel::InodeKind::RegularFile => {
                let fnm = String::from(ex_dir) + "/" + &name;
                let mut f = File::create(fnm.clone()).unwrap();
                f.write_all(&parcel.read(pf, ino, 0, None).unwrap())
                    .unwrap();
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
            pyxis_parcel::InodeKind::Symlink => {
                let fnm = String::from(ex_dir) + "/" + &name;
                std::os::unix::fs::symlink(
                    Path::new(std::ffi::OsStr::from_bytes(&parcel.readlink(ino).unwrap())),
                    fnm.clone(),
                )
                .unwrap();
            }
            pyxis_parcel::InodeKind::Char => {
                unimplemented!();
            }
        }
    }
}
