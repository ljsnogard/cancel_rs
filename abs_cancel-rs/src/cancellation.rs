use core::future::{self, IntoFuture};

/// An instance of [IntoFuture] for an async task that may or may not be
/// cancelled by an optional cancellation token.
///
/// Note: the lifetime here is required by `rustc` when implementing
/// [TrMayCancel] for your type. Along with future release of rustc, the `<'a>`
/// may be removed.
pub trait TrMayCancel<'a>
where
    Self: 'a + IntoFuture<Output = Self::MayCancelOutput>
{
    type MayCancelFuture<'f, C>: IntoFuture<Output = Self::MayCancelOutput>
    where
        'f: 'a,
        Self: 'f,
        C: 'f + TrCancellationToken;

    type MayCancelOutput;

    fn may_cancel_with<C>(
        self,
        cancel: C,
    ) -> Self::MayCancelFuture<'a, C>
    where
        C: 'a + TrCancellationToken;
}


/// A cancellation token can receive cancellation signal. Cloning the token
/// will also clone the receiver of the cancellation signal, and the cost of
/// cloning a cancellation token should be cheap.
pub trait TrCancellationToken
where
    Self: Send + Sync,
{
    type Cancellation: Future;
    type ChildToken: TrCancellationToken + Sized;

    /// Tests whether this token has received cancellation signal or not.
    fn is_cancelled(&self) -> bool;

    /// Tests whether this token will receive cancellation signal or not.
    fn can_be_cancelled(&self) -> bool;

    fn child_token(&self) -> Self::ChildToken;

    /// Creates a future that will become ready when the cancellation signal is
    /// received by this token.
    fn cancellation(self) -> Self::Cancellation;
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// CancelledToken
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// A token that is already cancelled and will never reset.
#[derive(Clone, Copy, Debug, Default)]
pub struct CancelledToken;

impl CancelledToken {
    #[allow(static_mut_refs)]
    pub fn shared_mut() -> &'static mut CancelledToken {
        static mut SHARED: CancelledToken = CancelledToken::new();
        unsafe { &mut SHARED }
    }

    /// Create an instance of `CancelledToken`
    pub const fn new() -> Self {
        CancelledToken
    }

    /// Always true
    pub const fn is_cancelled(&self) -> bool {
        true
    }
    /// Always false
    pub const fn can_be_cancelled(&self) -> bool {
        false
    }

    pub const fn child_token(&self) -> CancelledToken {
        CancelledToken::new()
    }

    /// Always return a ready future.
    pub fn cancellation(&mut self) -> future::Ready<()> {
        future::ready(())
    }
}

impl TrCancellationToken for CancelledToken {
    type Cancellation = future::Ready<()>;
    type ChildToken = CancelledToken;

    #[inline]
    fn is_cancelled(&self) -> bool {
        CancelledToken::is_cancelled(self)
    }

    #[inline]
    fn can_be_cancelled(&self) -> bool {
        CancelledToken::can_be_cancelled(self)
    }

    #[inline]
    fn child_token(&self) -> Self::ChildToken {
        CancelledToken::child_token(self)
    }

    #[inline]
    fn cancellation(mut self) -> Self::Cancellation {
        CancelledToken::cancellation(&mut self)
    }
}


//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// NonCancellableToken
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// A cancellation token that will never be cancelled, usually used
/// as a dummy for `TrCancellationToken`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NonCancellableToken;

impl NonCancellableToken {
    #[allow(static_mut_refs)]
    pub fn shared_mut() -> &'static mut NonCancellableToken {
        static mut SHARED: NonCancellableToken = NonCancellableToken::new();
        unsafe { &mut SHARED }
    }

    pub const fn new() -> Self {
        NonCancellableToken
    }

    /// Always false
    pub const fn is_cancelled(&self) -> bool {
        false
    }

    /// Always false
    pub const fn can_be_cancelled(&self) -> bool {
        false
    }

    pub const fn child_token(&self) -> NonCancellableToken {
        NonCancellableToken::new()
    }

    /// Always returns a pending future.
    pub fn cancellation(&mut self) -> future::Pending<()> {
        future::pending()
    }
}

impl TrCancellationToken for NonCancellableToken {
    type Cancellation = future::Pending<()>;
    type ChildToken = NonCancellableToken;

    #[inline]
    fn is_cancelled(&self) -> bool {
        NonCancellableToken::is_cancelled(self)
    }

    #[inline]
    fn can_be_cancelled(&self) -> bool {
        NonCancellableToken::can_be_cancelled(self)
    }

    #[inline]
    fn child_token(&self) -> Self::ChildToken {
        NonCancellableToken::child_token(self)
    }

    #[inline]
    fn cancellation(mut self) -> Self::Cancellation {
        NonCancellableToken::cancellation(&mut self)
    }
}

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// impl TrMayCancel for core::future::Ready
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

impl<'a, T> TrMayCancel<'a> for core::future::Ready<T>
where
    T: 'a,
{
    type MayCancelFuture<'lt_fut__, TyTok__> = core::future::Ready<T>
    where
        'lt_fut__: 'a,
        Self: 'lt_fut__,
        TyTok__: 'lt_fut__ + TrCancellationToken;

    type MayCancelOutput = T;

    fn may_cancel_with<C>(
        self,
        _tok: C,
    ) -> Self::MayCancelFuture<'a, C>
    where
        C: 'a + TrCancellationToken,
    {
        self
    }
}


#[cfg(test)]
mod tests_ {
    use crate::cancellation::{CancelledToken, NonCancellableToken};

    fn assure_send<T: Send>(t: T) -> T { t }

    fn assure_sync<T: Sync>(t: T) -> T { t }

    #[test]
    fn non_cancellable_token_shared_mut_should_be_send_and_sync() {
        let tok = NonCancellableToken::new();
        let tok = assure_send(tok);
        let _ = assure_sync(tok);
    }

    #[test]
    fn cancelled_token_shared_mut_should_be_send_and_sync() {
        let tok = CancelledToken::new();
        let tok = assure_send(tok);
        let _ = assure_sync(tok);
    }
}
