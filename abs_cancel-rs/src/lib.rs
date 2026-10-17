#![no_std]

mod cancellation;

#[cfg(test)]
extern crate std;

#[cfg(feature = "ok_or")]
pub mod ok_or;

pub use cancellation::{
    CancelledToken, NonCancellableToken,
    TrCancellationToken, TrMayCancel,
};
