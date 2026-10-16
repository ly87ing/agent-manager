//! Elevated fixed-path publication and current-instance repair, never app state.
use super::{
    repair_linux_ntp as native, repair_linux_ntp_file::HostLock, repair_linux_ntp_lease as lease,
    repair_linux_ntp_protocol as protocol,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

pub(super) fn observe(session: &native::Session) -> Option<native::State> {
    zbus::block_on(futures_lite::future::race(session.snapshot(), async {
        async_io::Timer::after(Duration::from_secs(8)).await;
        None
    }))
}

fn next(
    receiver: &mpsc::Receiver<protocol::Request>,
    lease: &lease::Lease,
) -> Option<protocol::Request> {
    loop {
        if !lease.fresh() {
            return None;
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(message) => return lease.fresh().then_some(message),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return None,
        }
    }
}

fn execute(args: lease::Arguments) -> Option<i32> {
    let pipe = Arc::new(AtomicBool::new(true));
    let (sender, receiver) = mpsc::sync_channel(2);
    let reading = pipe.clone();
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let deadline = protocol::wait_window();
        // Continue watching EOF while the main thread is inspecting or writing.
        loop {
            let message =
                protocol::receive(&mut input, deadline, || reading.load(Ordering::Acquire));
            if message.is_none() || sender.try_send(message.unwrap()).is_err() {
                break;
            }
        }
        reading.store(false, Ordering::Release);
    });
    let Some(lease) = lease::Lease::open(args, pipe) else {
        return Some(69);
    };
    let session = zbus::block_on(futures_lite::future::race(native::Session::open(), async {
        async_io::Timer::after(Duration::from_secs(8)).await;
        None
    }));
    let Some(session) = session else {
        return Some(69);
    };
    if zbus::block_on(session.root_sender()).is_none() {
        return Some(69);
    }
    let Some(before) = observe(&session) else {
        return Some(69);
    };
    if !before.available(lease.server()) {
        return Some(69);
    }
    let Some(executable) = session.executable(&before) else {
        return Some(69);
    };
    let lock = HostLock::acquire(&before.configuration)?;
    let ready = |expected: &native::State| {
        lease.fresh()
            && session.executable(expected).as_ref() == Some(&executable)
            && observe(&session).as_ref() == Some(expected)
            && lease.fresh()
    };
    if !ready(&before) {
        return Some(74);
    }
    let base = before.digest()?;
    let mut output = std::io::stdout();
    if !protocol::send(
        &mut output,
        &protocol::Response::Ready {
            version: protocol::VERSION,
            base: base.clone(),
        },
    ) {
        return Some(74);
    }
    match next(&receiver, &lease)? {
        protocol::Request::Commit {
            version: protocol::VERSION,
            base: requested,
        } if protocol::digest(&requested) && requested == base => {}
        _ => return Some(74),
    }
    if !ready(&before) {
        return Some(74);
    }
    let publication = lock.publish(&before.configuration, lease.server(), |configuration| {
        let mut expected = before.clone();
        expected.configuration = configuration.clone();
        ready(&expected)
    });
    let projected = publication.project(&before.configuration, lease.server());
    let mut after = projected.map(|configuration| {
        let mut after = before.clone();
        after.configuration = configuration;
        after
    });
    let mut runtime = "not_applied";
    if publication.durable && matches!(publication.status.as_str(), "saved" | "unchanged") {
        if let Some(expected) = after.as_ref().filter(|expected| ready(expected)) {
            let dispatch = zbus::block_on(futures_lite::future::race(
                session.set_runtime(expected, lease.server(), || {
                    lease.fresh() && session.executable(expected).as_ref() == Some(&executable)
                }),
                async {
                    async_io::Timer::after(Duration::from_secs(8)).await;
                    native::Dispatch::Unknown
                },
            ));
            runtime = match dispatch {
                native::Dispatch::Written => "written",
                native::Dispatch::Unchanged => "unchanged",
                native::Dispatch::Refused => "failed",
                native::Dispatch::Stale => "stale",
                native::Dispatch::Unknown => "outcome_unknown",
            };
            // A lost response can still have changed the runtime list. Project
            // only the requested list, then compare; uncertainty stays partial.
            if matches!(
                dispatch,
                native::Dispatch::Written | native::Dispatch::Unchanged | native::Dispatch::Unknown
            ) {
                after = expected.with_runtime(lease.server());
            }
        } else {
            runtime = "stale";
        }
    }
    let after_digest = after
        .as_ref()
        .filter(|after| ready(after))
        .and_then(native::State::digest);
    if !protocol::send(
        &mut output,
        &protocol::Response::Done {
            version: protocol::VERSION,
            publication,
            runtime: runtime.into(),
            after: after_digest.clone(),
        },
    ) {
        return Some(74);
    }
    let Some(after_digest) = after_digest else {
        return Some(74);
    };
    match next(&receiver, &lease) {
        Some(protocol::Request::Ack {
            version: protocol::VERSION,
            after: acknowledged,
        }) if protocol::digest(&acknowledged)
            && acknowledged == after_digest
            && after.as_ref().is_some_and(|after| ready(after)) =>
        {
            Some(0)
        }
        _ => Some(74),
    }
}

pub(super) fn run(args: &[String]) -> i32 {
    let Some(args) = lease::arguments(args) else {
        return 2;
    };
    if !super::clock_helper::boot_ms()
        .is_some_and(|now| super::clock_helper::valid_window(now, args.deadline))
    {
        return 74;
    }
    execute(args).unwrap_or(74)
}
