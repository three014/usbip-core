pub use error::Error;
use std::sync::atomic::AtomicUsize;
pub type Result<T, E> = std::result::Result<T, Error<E>>;

mod error {
    use std::fmt;

    /// The error type for the singleton module.
    /// Allows the `singleton::try_init` function
    /// to return an error of the user's choice
    /// should their initialization function fail.
    #[derive(Debug)]
    pub enum Error<E>
    where
        E: std::error::Error,
    {
        AlreadyInit,
        AlreadyFailed,
        UserSpecified(E),
    }

    impl<E> fmt::Display for Error<E>
    where
        E: std::error::Error,
    {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::AlreadyInit => write!(f, "singleton already initialized"),
                Error::UserSpecified(user) => write!(f, "{user}"),
                Error::AlreadyFailed => write!(f, "initialization had already failed"),
            }
        }
    }

    impl<E> std::error::Error for Error<E> where E: std::error::Error {}
}

pub const UNINITIALIZED: usize = 0;
pub const INITIALIZING: usize = 1;
pub const INITIALIZED: usize = 2;
pub const TERMINATING: usize = 3;
pub const ERROR: usize = 4;

/// Attempts to initialize the singleton using the
/// provided `init` function, keeping synchronization
/// with the `state` variable.
///
/// On first init, `state` should be set to `singleton::UNINITIALIZED` or else
/// `try_init` will never call `init` or worse, loop forever.
pub fn try_init<F, T, E>(state: &AtomicUsize, init: F) -> Result<T, E>
where
    F: FnOnce() -> std::result::Result<T, E>,
    E: std::error::Error,
{
    use std::sync::atomic::Ordering;
    let old_state = match state.compare_exchange(
        UNINITIALIZED,
        INITIALIZING,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(s) | Err(s) => s,
    };

    match old_state {
        UNINITIALIZED => {
            let value = init()
                .inspect_err(|_| {
                    state.store(ERROR, Ordering::SeqCst);
                })
                .map_err(|err| Error::UserSpecified(err))?;
            state.store(INITIALIZED, Ordering::SeqCst);
            Ok(value)
        }
        INITIALIZING => {
            while state.load(Ordering::SeqCst) == INITIALIZING {
                std::hint::spin_loop();
            }
            Err(Error::AlreadyInit)
        }
        ERROR => Err(Error::AlreadyFailed),
        _ => Err(Error::AlreadyInit),
    }
}

pub fn terminate<F>(state: &AtomicUsize, terminate: F)
where
    F: FnOnce(),
{
    use std::sync::atomic::Ordering;
    let old_state = match state.compare_exchange(
        INITIALIZED,
        TERMINATING,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(s) | Err(s) => s,
    };
    match old_state {
        INITIALIZED => {
            terminate();
            state.store(UNINITIALIZED, Ordering::SeqCst);
        }
        TERMINATING => {
            while state.load(Ordering::SeqCst) == TERMINATING {
                std::hint::spin_loop();
            }
        }
        _ => (),
    }
}
