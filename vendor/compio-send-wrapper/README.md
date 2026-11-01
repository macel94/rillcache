SendWrapper
===========

This [Rust] crate implements a wrapper type called `SendWrapper` which allows you to move around non-[`Send`] types
between threads, as long as you access the contained value only from within the original thread. You also have to
make sure that the wrapper is dropped from within the original thread. If any of these constraints is violated,
a panic occurs.

# Examples

```rust
use compio_send_wrapper::SendWrapper;
use std::rc::Rc;
use std::thread;
use std::sync::mpsc::channel;

// Rc is a non-Send type.
let value = Rc::new(42);

// We now wrap the value with `SendWrapper` (value is moved inside).
let wrapped_value = SendWrapper::new(value);

// A channel allows us to move the wrapped value between threads.
let (sender, receiver) = channel();

let t = thread::spawn(move || {

	// This would panic (because of accessing in the wrong thread):
	// let value = wrapped_value.get().unwrap();

	// Move SendWrapper back to main thread, so it can be dropped from there.
	// If you leave this out the thread will panic because of dropping from wrong thread.
	sender.send(wrapped_value).unwrap();

});

let wrapped_value = receiver.recv().unwrap();

// Now you can use the value again.
let value = wrapped_value.get().unwrap();

// alternatives for mutable dereferencing (value and wrapped_value must be mutable too, then):
// let mut value = wrapped_value.get_mut().unwrap();
```


## Wrapping `Future`s and `Stream`s

To use `SendWrapper` on `Future`s or `Stream`s, you should enable the Cargo feature `futures` first:
```toml
compio-send-wrapper = { version = "0.7", features = ["futures"] }
```

Then, you can transparently wrap your `Future` or `Stream`:
```rust
use futures::{executor, future::{self, BoxFuture}};
use compio_send_wrapper::SendWrapper;

// `Rc` is a `!Send` type,
let value = Rc::new(42);
// so this `Future` is `!Send` too as increments `Rc`'s inner counter.
let future = future::lazy(|_| value.clone());

// We now wrap the `future` with `SendWrapper` (value is moved inside),
let wrapped_future = SendWrapper::new(future);
// so now it's `Send` + `Sync` (`BoxFuture` trait object contains `Send` requirement).
let boxed_future: BoxFuture<_> = Box::pin(wrapped_future);

let t = thread::spawn(move || {
	// This would panic (because `future` is polled in wrong thread):
	// executor::block_on(boxed_future)
});
```


# Changelog

See [CHANGELOG.md](CHANGELOG.md)

# License

`compio-send-wrapper` is distributed under the terms of both the MIT license and the Apache License (Version 2.0).

See LICENSE-APACHE, and LICENSE-MIT for details.


[Rust]: https://www.rust-lang.org
[`Send`]: https://doc.rust-lang.org/std/marker/trait.Send.html
