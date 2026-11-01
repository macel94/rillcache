//! [`Future`] and [`Stream`] support for [`SendWrapper`].
use std::{future::Future, pin::Pin, task};

use futures_core::Stream;

use crate::{SendWrapper, invalid_deref, invalid_poll};

impl<F: Future + ?Sized> Future for SendWrapper<F> {
    type Output = F::Output;

    /// Polls this [`SendWrapper`] [`Future`].
    ///
    /// # Panics
    ///
    /// Polling panics if it is done from a different thread than the one the
    /// [`SendWrapper`] instance has been created with.
    #[track_caller]
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.get_pinned_mut()
            .unwrap_or_else(|| invalid_poll())
            .poll(cx)
    }
}

impl<S: Stream + ?Sized> Stream for SendWrapper<S> {
    type Item = S::Item;

    /// Polls this [`SendWrapper`] [`Stream`].
    ///
    /// # Panics
    ///
    /// Polling panics if it is done from a different thread than the one the
    /// [`SendWrapper`] instance has been created with.
    #[track_caller]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        self.get_pinned_mut()
            .unwrap_or_else(|| invalid_poll())
            .poll_next(cx)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.get().unwrap_or_else(|| invalid_deref()).size_hint()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use futures_executor as executor;
    use futures_util::{StreamExt, future, stream};

    use crate::SendWrapper;

    #[test]
    fn test_future() {
        let w1 = SendWrapper::new(future::ready(42));
        let w2 = w1.clone();
        assert_eq!(
            format!("{:?}", executor::block_on(w1)),
            format!("{:?}", executor::block_on(w2)),
        );
    }

    #[test]
    fn test_future_panic() {
        let w = SendWrapper::new(future::ready(42));
        let t = thread::spawn(move || executor::block_on(w));
        assert!(t.join().is_err());
    }

    #[test]
    fn test_stream() {
        let mut w1 = SendWrapper::new(stream::once(future::ready(42)));
        let mut w2 = SendWrapper::new(stream::once(future::ready(42)));
        assert_eq!(
            format!("{:?}", executor::block_on(w1.next())),
            format!("{:?}", executor::block_on(w2.next())),
        );
    }

    #[test]
    fn test_stream_panic() {
        let mut w = SendWrapper::new(stream::once(future::ready(42)));
        let t = thread::spawn(move || executor::block_on(w.next()));
        assert!(t.join().is_err());
    }
}
