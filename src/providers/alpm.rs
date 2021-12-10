use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use itertools::Itertools;
use lazy_static::lazy_static;
use pyxis_parcel::{InodeAttr, Parcel};

use crate::{exists_parcel, get_parcel_path, ParcelProvider};

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

pub fn get_deps(package: &str) -> Vec<String> {
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
    .iter()
    .map(|x| alpm_find_satisfier(x)[0].clone())
    .unique()
    .collect()
}

pub fn alpm_get_version(package: &str) -> Vec<String> {
    let pkb = Box::new(package.to_owned());
    with_alpm(Box::new(|alpm: &alpm::Alpm| {
        let mut res = Vec::new();
        let package = *pkb;
        for db in alpm.syncdbs() {
            if let Ok(pkg) = db.pkg(package.clone()) {
                res = vec![pkg.version().as_str().to_owned()];
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
        "http://archrepo.calamityconductor.com/{}/os/x86_64/{}",
        repo, filename
    ))
    .unwrap();

    let mut file = tempfile::tempfile().unwrap();

    let mut transfer = easy.transfer();
    transfer
        .write_function(|data| {
            file.write_all(data).unwrap();
            Ok(data.len())
        })
        .unwrap();
    transfer.perform().unwrap();
    std::mem::drop(transfer);

    file.seek(SeekFrom::Start(0)).unwrap();

    (file, filename.split('.').next_back().unwrap().to_owned())
}

fn parcel_from_pacman<R: Sized + std::io::Read>(
    provider: ParcelProvider,
    package: &str,
    mut archive: tar::Archive<R>,
) {
    let mut dir_map: BTreeMap<PathBuf, u64> = BTreeMap::new();
    dir_map.insert(PathBuf::from("/"), 1);

    let mut parcel = Parcel::new();

    parcel.metadata.depends = get_deps(package)
        .iter()
        .map(|x| String::from("arch|") + x)
        .collect();
    parcel.metadata.version = alpm_get_version(package)[0].clone();

    let time = std::time::SystemTime::now();
    let attr = InodeAttr {
        atime: time,
        ctime: time,
        mtime: time,
        uid:   0,
        gid:   0,
        nlink: 1,
        perm:  0o644,
        rdev:  0,
    };
    let pyxis_dir = parcel.add_directory(attr, BTreeMap::new());
    let provider_dir = parcel.add_directory(attr, BTreeMap::new());
    let parcel_dir = parcel.add_directory(attr, BTreeMap::new());
    parcel
        .insert_dirent(1, std::ffi::OsString::from(".PYXIS"), pyxis_dir)
        .unwrap();
    parcel
        .insert_dirent(pyxis_dir, std::ffi::OsString::from("arch"), provider_dir)
        .unwrap();
    parcel
        .insert_dirent(provider_dir, std::ffi::OsString::from(package), parcel_dir)
        .unwrap();

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
                let attr = InodeAttr {
                    atime: time,
                    ctime: time,
                    mtime: time,
                    uid:   ent_header.uid().unwrap() as u32,
                    gid:   ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm:  ent_header.mode().unwrap(),
                    rdev:  0,
                };
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                let ino = parcel
                    .add_file(pyxis_parcel::FileAdd::Bytes(buf), attr, BTreeMap::new())
                    .unwrap();
                match (parent_inode, entry_name.to_str().unwrap()) {
                    (1, ".INSTALL" | ".BUILDINFO" | ".MTREE" | ".PKGINFO") => parcel
                        .insert_dirent(parcel_dir, entry_name.to_owned(), ino)
                        .unwrap(),
                    _ => parcel
                        .insert_dirent(parent_inode, entry_name.to_owned(), ino)
                        .unwrap(),
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
                let ino = parcel.add_hardlink(link_name).unwrap();
                parcel
                    .insert_dirent(parent_inode, entry_name.to_owned(), ino)
                    .unwrap();
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
                    uid:   ent_header.uid().unwrap() as u32,
                    gid:   ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm:  ent_header.mode().unwrap(),
                    rdev:  0,
                };
                let ino = parcel
                    .add_symlink(link_name, attr, BTreeMap::new())
                    .unwrap();
                parcel
                    .insert_dirent(parent_inode, entry_name.to_owned(), ino)
                    .unwrap();
            }
            [b'5'] => {
                let attr = pyxis_parcel::InodeAttr {
                    atime: time,
                    ctime: time,
                    mtime: time,
                    uid:   ent_header.uid().unwrap() as u32,
                    gid:   ent_header.gid().unwrap() as u32,
                    nlink: 1,
                    perm:  ent_header.mode().unwrap(),
                    rdev:  0,
                };
                let ino = parcel.add_directory(attr, BTreeMap::new());
                parcel
                    .insert_dirent(parent_inode, entry_name.to_owned(), ino)
                    .unwrap();
                dir_map.insert(entry_path, ino);
            }
            _ => unimplemented!("TF:{:?}", header.typeflag),
        }
    }
    let parcelpath = get_parcel_path(provider, package);
    std::fs::create_dir_all(parcelpath.parent().unwrap()).unwrap();
    let file = File::create(parcelpath).unwrap();
    parcel.store(file).unwrap();
}

pub fn parcel_build(package: &str) {
    let package = &alpm_find_satisfier(package)[0];

    if exists_parcel(ParcelProvider::Arch, package) {
        return;
    }

    let (f, ext) = alpm_fetch(package);

    match ext.as_str() {
        "zst" => {
            let dec = zstd::stream::read::Decoder::new(f).unwrap();
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(ParcelProvider::Arch, package, archive);
        }
        "xz" => {
            let dec = xz::read::XzDecoder::new(f);
            let archive = tar::Archive::new(dec);
            parcel_from_pacman(ParcelProvider::Arch, package, archive);
        }
        _ => unimplemented!("{}", ext),
    };
}
