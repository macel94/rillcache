use super::super::mailbox::{ByteBudget, OwnerSender, ReplyReceiver, owner_mailbox, reply_channel};
use super::super::{Auth, ShardExecutor, SystemClock};
use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

fn services() -> Arc<ServerServices> {
    Arc::new(ServerServices::with_clock_and_auth(
        1024 * 1024,
        Arc::new(SystemClock),
        Auth::default(),
    ))
}

fn key_for(layout: PartitionLayout, owner: OwnerId) -> Vec<u8> {
    (0_u64..100_000)
        .map(|n| format!("key:{n}").into_bytes())
        .find(|key| layout.owner_for(key) == owner)
        .unwrap()
}

fn run<F: Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, future)
}

async fn ack(
    sender: &OwnerSender<DataWork, Control>,
    replies: &ByteBudget,
    make: impl FnOnce(Acknowledge) -> Control,
) -> Result<(), OwnerError> {
    let (answer, reply) = reply_channel(replies.reserve(0).await.unwrap());
    sender.control.reserve(0).await.unwrap().send_with(|| make(answer));
    *reply.recv().await.unwrap()
}

async fn command(
    sender: &OwnerSender<DataWork, Control>,
    replies: &ByteBudget,
    args: &[&[u8]],
) -> Vec<u8> {
    let (answer, reply) = reply_channel(replies.reserve(128).await.unwrap());
    let bytes = args.iter().map(|arg| arg.len()).sum();
    sender.data.reserve(bytes).await.unwrap().send_with(|| {
        let args: Vec<Vec<u8>> = args.iter().map(|arg| arg.to_vec()).collect();
        Box::new(move |executor: &ShardExecutor| {
            let args: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
            let mut output = Vec::new();
            executor.execute(&args, &mut output);
            let _ = answer.send(output);
        }) as DataWork
    });
    reply.recv().await.unwrap().to_vec()
}

#[test]
fn reservation_decision_apply_release_is_exclusive_and_exactly_once() {
    let layout = PartitionLayout::new(3).unwrap();
    let owner = layout.owners().next().unwrap();
    let core = OwnerCore::new(owner, layout, services()).unwrap();
    let key = key_for(layout, owner);
    let mut output = Vec::new();
    core.try_execute(&[b"SET", &key, b"old"], &mut output).unwrap();
    assert_eq!(output, b"+OK\r\n");
    let id = ReservationId::new(1).unwrap();
    let other = ReservationId::new(2).unwrap();
    assert_eq!(ReservationId::new(0), Err(OwnerError::InvalidReservation));
    core.reserve(id).unwrap();
    assert_eq!(core.reserve(other), Err(OwnerError::Fenced));
    output.clear();
    assert_eq!(
        core.try_execute(&[b"GET", &key], &mut output),
        Err(OwnerError::Fenced)
    );
    assert!(output.is_empty());
    assert_eq!(
        core.read_reserved(other, |data| data.entries.len()),
        Err(OwnerError::WrongReservation)
    );
    assert_eq!(core.read_reserved(id, |data| data.entries.len()), Ok(1));
    assert_eq!(core.apply(id, |_| Ok(())), Err(OwnerError::NotDecided));
    assert_eq!(core.release(id), Err(OwnerError::NotApplied));
    core.decide(id).unwrap();
    core.decide(id).unwrap();
    assert_eq!(core.abort(id), Err(OwnerError::DecisionRecorded));
    assert_eq!(core.stop(), Err(OwnerError::DecisionRecorded));
    let applied = AtomicUsize::new(0);
    core.apply(id, |data| {
        applied.fetch_add(1, Ordering::SeqCst);
        let total = data.payload_bytes + key.len() + 3;
        data.put(&key, b"new", None, total, 0);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        core.apply(id, |_| {
            applied.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        Err(OwnerError::AlreadyApplied)
    );
    assert_eq!(applied.load(Ordering::SeqCst), 1);
    assert_eq!(
        core.try_execute(&[b"GET", &key], &mut output),
        Err(OwnerError::Fenced)
    );
    core.release(id).unwrap();
    output.clear();
    core.try_execute(&[b"GET", &key], &mut output).unwrap();
    assert_eq!(output, b"$3\r\nnew\r\n");
    assert_eq!(core.release(id), Err(OwnerError::WrongReservation));
    let foreign = key_for(layout, layout.owners().nth(1).unwrap());
    assert_eq!(
        core.try_execute(&[b"SET", &foreign, b"wrong"], &mut Vec::new()),
        Err(OwnerError::WrongOwner)
    );
    assert_eq!(
        core.try_execute(&[b"FLUSHALL"], &mut Vec::new()),
        Err(OwnerError::NeedsCoordinator)
    );
    core.stop().unwrap();
    assert_eq!(core.turn(|_| ()), Err(OwnerError::Stopped));
}

#[test]
fn local_waiter_has_no_data_borrow_and_wakes_after_apply_and_release() {
    run(async {
        let layout = PartitionLayout::new(1).unwrap();
        let owner = layout.owners().next().unwrap();
        let core = OwnerCore::new(owner, layout, services()).unwrap();
        let id = ReservationId::new(1).unwrap();
        core.reserve(id).unwrap();
        let mut output = Vec::new();
        {
            let mut read = std::pin::pin!(core.execute(&[b"GET", b"key"], &mut output));
            let mut cx = Context::from_waker(Waker::noop());
            assert_eq!(read.as_mut().poll(&mut cx), Poll::Pending);
            assert!(core.executor.data.try_borrow_mut().is_ok());
            core.decide(id).unwrap();
            core.apply(id, |data| {
                data.put(b"key", b"value", None, 8, 0);
                Ok(())
            })
            .unwrap();
            assert_eq!(read.as_mut().poll(&mut cx), Poll::Pending);
            core.release(id).unwrap();
            assert_eq!(read.as_mut().poll(&mut cx), Poll::Ready(Ok(Action::Continue)));
        }
        assert_eq!(output, b"$5\r\nvalue\r\n");
    });
}

#[test]
fn cancelled_grant_does_not_strand_a_fence_or_block_stop() {
    run(async {
        let layout = PartitionLayout::new(1).unwrap();
        let owner = layout.owners().next().unwrap();
        let (sender, inbox) = owner_mailbox::<DataWork, Control>(1, 1024, 2, 1024).unwrap();
        let core = Rc::new(OwnerCore::new(owner, layout, services()).unwrap());
        let actor = OwnerActor::new(Rc::clone(&core), inbox);
        let replies = ByteBudget::new(1024).unwrap();
        let (answer, reply) = reply_channel(replies.reserve(0).await.unwrap());
        sender.control.reserve(0).await.unwrap().send_with(|| Control::Reserve {
            id: ReservationId::new(1).unwrap(),
            reply: answer,
        });
        drop(reply);
        let actor = tokio::task::spawn_local(actor.run());
        ack(&sender, &replies, |reply| Control::Stop { reply })
            .await
            .unwrap();
        actor.await.unwrap().unwrap();
        assert_eq!(core.phase.get(), Phase::Stopped);
    });
}

#[test]
fn apply_finishes_when_response_is_cancelled_and_release_unblocks_queued_data() {
    run(async {
        let layout = PartitionLayout::new(1).unwrap();
        let owner = layout.owners().next().unwrap();
        let (sender, inbox) = owner_mailbox::<DataWork, Control>(1, 1024, 1, 1024).unwrap();
        let core = Rc::new(OwnerCore::new(owner, layout, services()).unwrap());
        let actor = tokio::task::spawn_local(OwnerActor::new(Rc::clone(&core), inbox).run());
        let replies = ByteBudget::new(1024).unwrap();
        let id = ReservationId::new(1).unwrap();
        ack(&sender, &replies, |reply| Control::Reserve { id, reply })
            .await
            .unwrap();
        ack(&sender, &replies, |reply| Control::Decide { id, reply })
            .await
            .unwrap();
        let applied = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&applied);
        let (answer, reply) = reply_channel(replies.reserve(0).await.unwrap());
        sender.control.reserve(0).await.unwrap().send_with(|| Control::Apply {
            id,
            now: 0,
            reply: answer,
            op: Box::new(move |executor, _now| {
                count.fetch_add(1, Ordering::SeqCst);
                let mut data = executor.data.borrow_mut();
                let total = data.payload_bytes + 3 + 5;
                data.put(b"key", b"value", None, total, 0);
                Ok(())
            }),
        });
        drop(reply);
        let mut output = Vec::new();
        {
            let read = core.execute(&[b"GET", b"key"], &mut output);
            let mut read = std::pin::pin!(read);
            assert!(
                read.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            ack(&sender, &replies, |reply| Control::Release { id, reply })
                .await
                .unwrap();
            assert_eq!(read.await, Ok(Action::Continue));
        }
        assert_eq!(applied.load(Ordering::SeqCst), 1);
        assert_eq!(output, b"$5\r\nvalue\r\n");
        ack(&sender, &replies, |reply| Control::Stop { reply })
            .await
            .unwrap();
        actor.await.unwrap().unwrap();
    });
}

#[test]
fn partial_apply_failure_is_fail_stop_not_partial_state_service() {
    let services = services();
    let layout = PartitionLayout::new(1).unwrap();
    let core = OwnerCore::new(
        layout.owners().next().unwrap(),
        layout,
        Arc::clone(&services),
    )
    .unwrap();
    let id = ReservationId::new(1).unwrap();
    core.reserve(id).unwrap();
    core.decide(id).unwrap();
    assert_eq!(
        core.apply(id, |data| {
            data.put(b"key", b"partial", None, 10, 0);
            Err::<(), _>(OwnerError::Failed)
        }),
        Err(OwnerError::Failed)
    );
    assert_eq!(core.turn(|_| ()), Err(OwnerError::Failed));
    assert_eq!(core.release(id), Err(OwnerError::Failed));
    assert!(services.shutdown_requested());
    assert!(services.owner_failed.load(Ordering::Acquire));
}

#[test]
fn three_real_owner_threads_hold_disjoint_data_and_fence_partial_visibility() {
    let services = services();
    let layout = PartitionLayout::new(3).unwrap();
    let mut senders = Vec::new();
    let mut threads = Vec::new();
    for owner in layout.owners() {
        let (sender, inbox) = owner_mailbox::<DataWork, Control>(1, 1024, 2, 1024).unwrap();
        let services = Arc::clone(&services);
        threads.push(std::thread::spawn(move || {
            let core = Rc::new(OwnerCore::new(owner, layout, services).unwrap());
            assert_eq!(core.owner(), owner);
            run(OwnerActor::new(core, inbox).run())
        }));
        senders.push(sender);
    }
    run(async {
        let replies = ByteBudget::new(4096).unwrap();
        let keys: Vec<Vec<u8>> = layout.owners().map(|owner| key_for(layout, owner)).collect();
        for (sender, key) in senders.iter().zip(&keys) {
            assert_eq!(command(sender, &replies, &[b"SET", key, b"old"]).await, b"+OK\r\n");
        }
        let id = ReservationId::new(1).unwrap();
        for sender in &senders {
            ack(sender, &replies, |reply| Control::Reserve { id, reply })
                .await
                .unwrap();
        }
        for (index, sender) in senders.iter().enumerate() {
            let (answer, reply) = reply_channel(replies.reserve(0).await.unwrap());
            sender.control.reserve(0).await.unwrap().send_with(|| Control::Read {
                id,
                now: 0,
                reply: answer,
                op: Box::new(move |executor, _now| {
                    let data = executor.data.borrow();
                    assert_eq!(data.entries.len(), 1);
                    assert!(data.entries.keys().all(|key| layout.owner_for(key).index() == index));
                    let _ = reply.send(data.entries.len());
                }),
            });
            assert_eq!(*reply.recv().await.unwrap(), 1);
        }
        for sender in &senders {
            ack(sender, &replies, |reply| Control::Decide { id, reply })
                .await
                .unwrap();
        }
        for (index, sender) in senders.iter().enumerate() {
            let key = keys[index].clone();
            ack(sender, &replies, |reply| Control::Apply {
                id,
                now: 0,
                reply,
                op: Box::new(move |executor, _now| {
                    let mut data = executor.data.borrow_mut();
                    let total = data.payload_bytes + key.len() + 3;
                    data.put(&key, b"new", None, total, 0);
                    Ok(())
                }),
            })
            .await
            .unwrap();
            // The earlier owner's applied value is still inaccessible while
            // any participant remains unapplied/unreleased.
            let (answer, reply): (_, ReplyReceiver<Vec<u8>>) =
                reply_channel(replies.reserve(128).await.unwrap());
            let key = keys[0].clone();
            // Only queue once: the DATA lane intentionally has capacity one.
            if index == 0 {
                senders[0].data.reserve(key.len()).await.unwrap().send_with(|| {
                    Box::new(move |executor: &ShardExecutor| {
                        let mut output = Vec::new();
                        executor.execute(&[b"GET", &key], &mut output);
                        let _ = answer.send(output);
                    }) as DataWork
                });
                let mut pending = std::pin::pin!(reply.recv());
                assert!(
                    pending
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                // Dropping this response does not make the queued data execute
                // before release; it remains fenced and harmlessly discards it.
            }
        }
        for sender in &senders {
            ack(sender, &replies, |reply| Control::Release { id, reply })
                .await
                .unwrap();
        }
        for (sender, key) in senders.iter().zip(&keys) {
            assert_eq!(command(sender, &replies, &[b"GET", key]).await, b"$3\r\nnew\r\n");
        }
        // Also exercise an abort transition without ever deciding/applying.
        let next = ReservationId::new(2).unwrap();
        for sender in &senders {
            ack(sender, &replies, |reply| Control::Reserve { id: next, reply })
                .await
                .unwrap();
        }
        for sender in &senders {
            ack(sender, &replies, |reply| Control::Abort { id: next, reply })
                .await
                .unwrap();
            ack(sender, &replies, |reply| Control::Stop { reply })
                .await
                .unwrap();
        }
    });
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    let trace = services.owner_trace();
    let constructs: std::collections::HashSet<_> = trace
        .iter()
        .filter(|(kind, _)| *kind == "construct")
        .map(|(_, id)| *id)
        .collect();
    let drops: std::collections::HashSet<_> = trace
        .iter()
        .filter(|(kind, _)| *kind == "drop")
        .map(|(_, id)| *id)
        .collect();
    assert_eq!(constructs.len(), 3);
    assert_eq!(constructs, drops);
}
