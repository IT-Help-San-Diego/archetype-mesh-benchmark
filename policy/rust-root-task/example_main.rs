//
// Copyright 2023, Colias Group, LLC
// Modifications 2026, IT Help San Diego Inc.
//
// SPDX-License-Identifier: BSD-2-Clause
//
// calibration-scope seL4 root task — capability-confinement demonstration.
//
// WHAT CHANGED vs the upstream demo:
//   The stock demo mints ONE badged notification (badge 0x1337) and verifies the
//   badge round-trips. This version demonstrates the property that actually matters
//   for calibration-scope's thesis: seL4 BADGES let one object distinguish its
//   senders, and a capability confines what a holder can do. We mint TWO badged
//   capabilities from the SAME underlying notification, signal each in turn, and
//   verify each delivery carries its OWN sender identity. This is the kernel-level
//   analogue of the Genie-coefficient argument (DECISIONS: the harness bounds the
//   agent): a confined capability can only do what its badge authorizes.
//
//   The literal `TEST_PASS` marker is PRESERVED and still gates the run (test.py
//   asserts it within 3s), so this remains a valid boot-validation.
//
// !!! BUILD-GATED: this file has NOT been compiled or booted here (the EC2 builder
//     was stopped). It is written against the exact rust-sel4 API in the upstream
//     main.rs. DO NOT MERGE until it builds clean AND boots to TEST_PASS under QEMU
//     on the box, and the boot receipt is evicted per §4b. See the handoff note.

#![no_std]
#![no_main]

use sel4_root_task::{root_task, Never};

// Two distinct sender identities minted from ONE notification object.
const BADGE_A: sel4::Word = 0x1337;        // preserved from the demo (back-compat receipt)
const BADGE_B: sel4::Word = 0x5C09E;       // "SCOPE" — calibration-scope's own sender id

#[root_task]
fn main(bootinfo: &sel4::BootInfoPtr) -> sel4::Result<Never> {
    sel4::debug_println!("Hello, World!");

    let blueprint = sel4::ObjectBlueprint::Notification;

    // Pick an untyped region big enough to retype into a Notification.
    let chosen_untyped_ix = bootinfo
        .untyped_list()
        .iter()
        .position(|desc| !desc.is_device() && desc.size_bits() >= blueprint.physical_size_bits())
        .unwrap();
    let untyped = bootinfo.untyped().index(chosen_untyped_ix).cap();

    // We need THREE empty slots now: one unbadged notification + two badged mints.
    let mut empty_slots = bootinfo
        .empty()
        .range()
        .map(sel4::init_thread::Slot::from_index);
    let unbadged_notification_slot = empty_slots.next().unwrap();
    let badged_a_slot = empty_slots.next().unwrap();
    let badged_b_slot = empty_slots.next().unwrap();

    let cnode = sel4::init_thread::slot::CNODE.cap();

    // Create the backing notification object.
    untyped.untyped_retype(
        &blueprint,
        &cnode.absolute_cptr_for_self(),
        unbadged_notification_slot.index(),
        1,
    )?;

    // Mint TWO badged capabilities to the SAME notification, each carrying a
    // different sender identity. This is the capability-confinement primitive:
    // the holder of badge B cannot impersonate badge A.
    cnode.absolute_cptr(badged_a_slot.cptr()).mint(
        &cnode.absolute_cptr(unbadged_notification_slot.cptr()),
        sel4::CapRights::write_only(),
        BADGE_A,
    )?;
    cnode.absolute_cptr(badged_b_slot.cptr()).mint(
        &cnode.absolute_cptr(unbadged_notification_slot.cptr()),
        sel4::CapRights::write_only(),
        BADGE_B,
    )?;

    let unbadged_notification = unbadged_notification_slot
        .downcast::<sel4::cap_type::Notification>()
        .cap();
    let badged_a = badged_a_slot.downcast::<sel4::cap_type::Notification>().cap();
    let badged_b = badged_b_slot.downcast::<sel4::cap_type::Notification>().cap();

    // Deliver A, observe its identity in isolation.
    badged_a.signal();
    let (_, observed_a) = unbadged_notification.wait();
    sel4::debug_println!("sender A badge = {:#x}", observed_a);
    assert_eq!(observed_a, BADGE_A, "badge A did not round-trip");

    // Deliver B, observe its identity in isolation.
    badged_b.signal();
    let (_, observed_b) = unbadged_notification.wait();
    sel4::debug_println!("sender B badge = {:#x}", observed_b);
    assert_eq!(observed_b, BADGE_B, "badge B did not round-trip");

    // The confinement property: the two senders are distinguishable and neither
    // can forge the other's identity.
    assert_ne!(observed_a, observed_b, "distinct capabilities collided");
    sel4::debug_println!("capabilities confined: A={:#x} B={:#x} distinct", observed_a, observed_b);

    // PRESERVED marker — test.py gates on this literal string.
    sel4::debug_println!("TEST_PASS");

    sel4::init_thread::suspend_self()
}
