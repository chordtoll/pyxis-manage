use nix::{
    fcntl::OFlag,
    poll::{PollFd, PollFlags},
    sys::wait::WaitStatus,
    unistd::execv,
};

pub fn run_in_chroot(cmdline: String, input: String) -> i32 {
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
