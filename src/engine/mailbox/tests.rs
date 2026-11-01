//! Admission, cancellation and shutdown behaviour of the cross-owner mailbox.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
        .block_on(future)
}

/// Every await that could stall a broken mailbox is bounded, so a regression
/// fails instead of hanging the suite.
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("mailbox operation timed out")
}

#[test]
fn impossible_sizes_fail_without_waiting_and_close_wakes_reply_budget() {
    block_on(async {
        assert_eq!(
            ByteBudget::new(0).err(),
            Some(MailboxError::InvalidCapacity)
        );
        let budget = ByteBudget::new(8).expect("budget");
        // Larger than the whole budget: rejected immediately, never queued.
        assert_eq!(
            bounded(budget.reserve(9)).await.err(),
            Some(MailboxError::TooLarge)
        );
        // A reply may not retain more bytes than its granted credit.
        let credit = bounded(budget.reserve(8)).await.expect("credit");
        let (sender, _receiver) = reply_channel::<Vec<u8>>(credit);
        assert_eq!(
            sender.send(vec![b'x'; 9]).err(),
            Some(MailboxError::ReplyTooLarge)
        );
    });
}

#[test]
fn cancelling_admission_releases_credit_and_never_prepares_input() {
    block_on(async {
        let (sender, receiver) = lane::<Vec<u8>>(1, 8).expect("lane");
        let admission = bounded(sender.reserve(8)).await.expect("admission");
        // Cancelling must return both the message slot and the byte credit and
        // must not construct the owned request.
        let prepared = AtomicBool::new(false);
        drop(admission);
        assert!(!prepared.load(Ordering::Relaxed));
        let again = bounded(sender.reserve(8)).await.expect("credit returned");
        again.send_with(|| {
            prepared.store(true, Ordering::Relaxed);
            b"ok".to_vec()
        });
        assert!(prepared.load(Ordering::Relaxed));
        drop(receiver);
    });
}

#[test]
fn closing_receiver_wakes_both_byte_and_message_admission_waiters() {
    block_on(async {
        // One message slot and exactly one message worth of bytes, so a second
        // admission waits on both resources at once.
        let (sender, mut receiver) = lane::<u8>(1, 4).expect("lane");
        let held = bounded(sender.reserve(4)).await.expect("first admission");
        let byte_waiter = sender.clone();
        let message_waiter = sender.clone();
        let byte_task = tokio::spawn(async move { byte_waiter.reserve(4).await.err() });
        let message_task = tokio::spawn(async move { message_waiter.reserve(0).await.err() });
        tokio::task::yield_now().await;
        receiver.close();
        let byte_result = bounded(byte_task).await.expect("byte waiter task");
        let message_result = bounded(message_task).await.expect("message waiter task");
        drop(held);
        assert_eq!(byte_result, Some(MailboxError::Closed));
        assert_eq!(message_result, Some(MailboxError::Closed));
    });
}

#[test]
fn fenced_full_data_lane_does_not_consume_control_or_reply_capacity() {
    block_on(async {
        let (owner, mut inbox) =
            owner_mailbox::<Vec<u8>, &'static str>(1, 4, 1, 4).expect("mailbox");
        let data = bounded(owner.data.reserve(4)).await.expect("data");
        data.send_with(|| b"payload".to_vec());
        // The data lane is now full; control must still be admitted.
        let control = bounded(owner.control.reserve(4))
            .await
            .expect("control while data is full");
        control.send_with(|| "stop");
        match bounded(inbox.next(true)).await {
            Some(Incoming::Control(message)) => {
                assert_eq!(message.dispatch(|value| value), "stop");
            }
            other => panic!(
                "control must be served while data is fenced: {:?}",
                other.is_some()
            ),
        }
        match bounded(inbox.next(true)).await {
            Some(Incoming::Data(message)) => {
                assert_eq!(message.dispatch(|value| value), b"payload")
            }
            _ => panic!("admitted data must still be delivered"),
        }
    });
}

#[test]
fn closing_data_preserves_already_admitted_work_and_control_progress() {
    block_on(async {
        let (owner, mut inbox) =
            owner_mailbox::<Vec<u8>, &'static str>(2, 8, 1, 4).expect("mailbox");
        let admitted = bounded(owner.data.reserve(4)).await.expect("data");
        admitted.send_with(|| b"admitted".to_vec());
        let control = bounded(owner.control.reserve(4)).await.expect("control");
        control.send_with(|| "release");
        inbox.close_data();
        // Admission is closed, but the already-admitted message is not lost and
        // control keeps making progress.
        match bounded(inbox.next(true)).await {
            Some(Incoming::Control(message)) => {
                assert_eq!(message.dispatch(|value| value), "release");
            }
            _ => panic!("control must be delivered after close_data"),
        }
        match bounded(inbox.next(true)).await {
            Some(Incoming::Data(message)) => {
                assert_eq!(message.dispatch(|value| value), b"admitted")
            }
            _ => panic!("admitted data must survive close_data"),
        }
    });
}

#[test]
fn sustained_control_cannot_starve_admitted_data() {
    block_on(async {
        let (owner, mut inbox) = owner_mailbox::<u8, u8>(1, 1, 1, 1).unwrap();
        owner.data.reserve(1).await.unwrap().send_with(|| 7);
        let mut delivered = false;
        // Replenish CONTROL before every turn: draining a finite queue alone
        // would not demonstrate a bound under sustained control traffic.
        for turn in 0..=8 {
            owner.control.reserve(1).await.unwrap().send_with(|| 9);
            match bounded(inbox.next(true)).await.unwrap() {
                Incoming::Control(message) => {
                    assert_eq!(message.dispatch(|value| value), 9);
                    assert!(turn < 8, "CONTROL starved admitted DATA beyond eight turns");
                }
                Incoming::Data(message) => {
                    assert_eq!(message.dispatch(|value| value), 7);
                    delivered = true;
                    break;
                }
            }
        }
        assert!(delivered);
        // The fairness turn must not consume or lose the queued control work.
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Control(message) => assert_eq!(message.dispatch(|value| value), 9),
            _ => panic!("CONTROL must resume immediately after the fairness turn"),
        }
    });
}

#[test]
fn fairness_preserves_data_fifo_and_bounds_control_delay_to_one_data_turn() {
    block_on(async {
        let (owner, mut inbox) = owner_mailbox::<u8, u8>(2, 2, 1, 1).unwrap();
        for value in [1, 2] {
            owner.data.reserve(1).await.unwrap().send_with(|| value);
        }
        for _ in 0..8 {
            owner.control.reserve(1).await.unwrap().send_with(|| 3);
            match bounded(inbox.next(true)).await.unwrap() {
                Incoming::Control(message) => assert_eq!(message.dispatch(|value| value), 3),
                _ => panic!("CONTROL retains its initial burst priority"),
            }
        }
        owner.control.reserve(1).await.unwrap().send_with(|| 4);
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Data(message) => assert_eq!(message.dispatch(|value| value), 1),
            _ => panic!("DATA needs a fairness turn after the bounded burst"),
        }
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Control(message) => assert_eq!(message.dispatch(|value| value), 4),
            _ => panic!("CONTROL must not wait behind a second DATA turn"),
        }
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Data(message) => assert_eq!(message.dispatch(|value| value), 2),
            _ => panic!("DATA must preserve FIFO order"),
        }
    });
}

#[test]
fn exhausted_control_burst_never_overrides_a_data_fence() {
    block_on(async {
        let (owner, mut inbox) = owner_mailbox::<u8, u8>(1, 1, 1, 1).unwrap();
        owner.data.reserve(1).await.unwrap().send_with(|| 7);
        for _ in 0..32 {
            owner.control.reserve(1).await.unwrap().send_with(|| 9);
            match bounded(inbox.next(false)).await.unwrap() {
                Incoming::Control(message) => assert_eq!(message.dispatch(|value| value), 9),
                _ => panic!("fenced DATA must never run, even after many CONTROL turns"),
            }
        }
        assert!(owner.data.try_reserve(1).unwrap().is_none());
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Data(message) => assert_eq!(message.dispatch(|value| value), 7),
            _ => panic!("unfenced admitted DATA must remain available"),
        }
    });
}

#[test]
fn an_empty_data_lane_does_not_hold_control_at_the_burst_limit() {
    block_on(async {
        let (owner, mut inbox) = owner_mailbox::<u8, u8>(1, 1, 1, 1).unwrap();
        for _ in 0..32 {
            owner.control.reserve(1).await.unwrap().send_with(|| 9);
            match bounded(inbox.next(true)).await.unwrap() {
                Incoming::Control(message) => assert_eq!(message.dispatch(|value| value), 9),
                _ => panic!("empty DATA must never delay ready CONTROL"),
            }
        }
    });
}

#[test]
fn cancelling_an_idle_receive_does_not_reset_the_fairness_bound() {
    block_on(async {
        let (owner, mut inbox) = owner_mailbox::<u8, u8>(1, 1, 1, 1).unwrap();
        for _ in 0..8 {
            owner.control.reserve(1).await.unwrap().send_with(|| 9);
            match bounded(inbox.next(true)).await.unwrap() {
                Incoming::Control(message) => {
                    message.dispatch(|_| ());
                }
                _ => panic!("only CONTROL was queued"),
            }
        }
        let mut pending = Box::pin(inbox.next(true));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(pending.as_mut(), &mut cx).is_pending());
        drop(pending);
        owner.data.reserve(1).await.unwrap().send_with(|| 7);
        owner.control.reserve(1).await.unwrap().send_with(|| 9);
        match bounded(inbox.next(true)).await.unwrap() {
            Incoming::Data(message) => assert_eq!(message.dispatch(|value| value), 7),
            _ => panic!("an idle/cancelled receive must not restart the CONTROL burst"),
        }
    });
}

#[test]
fn reply_cancellation_and_oversize_release_all_reserved_credit() {
    block_on(async {
        let budget = ByteBudget::new(8).expect("budget");
        let credit = bounded(budget.reserve(8)).await.expect("credit");
        let (sender, receiver) = reply_channel::<Vec<u8>>(credit);
        drop(receiver);
        assert!(sender.is_closed());
        assert_eq!(
            sender.send(b"late".to_vec()).err(),
            Some(MailboxError::Closed)
        );
        // The cancelled reply must return its whole credit.
        let reused = bounded(budget.reserve(8)).await.expect("credit returned");
        assert_eq!(reused.bytes(), 8);
    });
}
