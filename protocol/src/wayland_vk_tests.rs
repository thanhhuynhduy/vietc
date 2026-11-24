// SPDX-License-Identifier: MIT
use super::*;
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

fn u32_ne(bytes: &[u8]) -> u32 {
    u32::from_ne_bytes(bytes[..4].try_into().unwrap())
}

fn wl_string(value: &str) -> Vec<u8> {
    let mut bytes = ((value.len() + 1) as u32).to_ne_bytes().to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    bytes.resize((bytes.len() + 3) & !3, 0);
    bytes
}

fn read_msg(sock: &mut UnixStream) -> (u32, u16, Vec<u8>) {
    let mut header = [0u8; 8];
    sock.read_exact(&mut header).unwrap();
    let obj = u32_ne(&header);
    let word = u32_ne(&header[4..]);
    let op = word as u16;
    let size = (word >> 16) as usize;
    let mut payload = vec![0u8; size - 8];
    sock.read_exact(&mut payload).unwrap();
    (obj, op, payload)
}

fn send_event(sock: &mut UnixStream, obj: u32, op: u16, args: &[u8]) {
    let mut pkt = obj.to_ne_bytes().to_vec();
    let header = (((args.len() + 8) as u32) << 16) | u32::from(op);
    pkt.extend_from_slice(&header.to_ne_bytes());
    pkt.extend_from_slice(args);
    sock.write_all(&pkt).unwrap();
}

#[test]
fn wire_virtual_keyboard_uploads_keymap_and_emits_atomic_vietnamese_replacement() {
    let (client, mut server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    server
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    let server_thread = thread::spawn(move || {
        // 1. Registry discovery + roundtrip
        let (disp1, op1, reg_args) = read_msg(&mut server);
        assert_eq!((disp1, op1), (1, 1));
        let registry_id = u32_ne(&reg_args);

        let (disp2, op2, sync1_args) = read_msg(&mut server);
        assert_eq!((disp2, op2), (1, 0));
        let sync1_id = u32_ne(&sync1_args);

        let mut seat_global = 10u32.to_ne_bytes().to_vec();
        seat_global.extend(wl_string("wl_seat"));
        seat_global.extend(7u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &seat_global);

        let mut vk_global = 20u32.to_ne_bytes().to_vec();
        vk_global.extend(wl_string("zwp_virtual_keyboard_manager_v1"));
        vk_global.extend(1u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &vk_global);

        send_event(&mut server, sync1_id, 0, &1u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync1_id.to_ne_bytes());

        // 2. Bind seat, bind vk manager, create_virtual_keyboard, keymap, modifiers, roundtrip
        let (r1, rop1, _) = read_msg(&mut server);
        assert_eq!((r1, rop1), (registry_id, 0));
        let (r2, rop2, bind_mgr_args) = read_msg(&mut server);
        assert_eq!((r2, rop2), (registry_id, 0));
        let mgr_id = u32_ne(&bind_mgr_args[bind_mgr_args.len() - 4..]);

        let (m_obj, m_op, create_vk_args) = read_msg(&mut server);
        assert_eq!((m_obj, m_op), (mgr_id, 0));
        let vk_id = u32_ne(&create_vk_args[4..]);

        let (k_obj, k_op, keymap_args) = read_msg(&mut server);
        assert_eq!((k_obj, k_op), (vk_id, 0));
        assert_eq!(u32_ne(&keymap_args), WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1);
        assert!(u32_ne(&keymap_args[4..]) > 1000);

        let (mod_obj, mod_op, _) = read_msg(&mut server);
        assert_eq!((mod_obj, mod_op), (vk_id, 2));

        let (disp3, op3, sync2_args) = read_msg(&mut server);
        assert_eq!((disp3, op3), (1, 0));
        let sync2_id = u32_ne(&sync2_args);
        send_event(&mut server, sync2_id, 0, &2u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync2_id.to_ne_bytes());

        // 3. Read atomic replacement: extra_deletes=1, backspaces=2, text="Việt"
        let mut key_ops = Vec::new();
        let mut mod_ops = Vec::new();
        loop {
            let (obj, op, args) = read_msg(&mut server);
            if obj == vk_id && op == 3 {
                // destroy on drop
                break;
            }
            if obj == vk_id && op == 1 {
                let key = u32_ne(&args[4..]);
                let state = u32_ne(&args[8..]);
                key_ops.push((key, state));
            } else if obj == vk_id && op == 2 {
                let depressed = u32_ne(&args[0..4]);
                let locked = u32_ne(&args[8..12]);
                mod_ops.push((depressed, locked));
            }
        }
        (key_ops, mod_ops)
    });

    let conn = Connection::from_socket(client).unwrap();
    let vk = WaylandVirtualKeyboard::from_connection(conn, false, true).unwrap();
    assert!(vk.is_healthy());
    assert!(WaylandVirtualKeyboard::can_encode_str(
        "Xin chào Việt Nam! Đỗ Ước 2026"
    ));
    assert!(!WaylandVirtualKeyboard::can_encode_str("🚀"));

    vk.set_num_lock(true);
    vk.set_caps_lock(true);
    // Held key press + hardware repeat (value=2) + release must not emit
    // synthetic release/press pairs (compositor handles native key repeat).
    assert!(vk.send_key_event(30, 1, false));
    assert!(vk.send_key_event(30, 2, false));
    assert!(vk.send_key_event(30, 0, false));
    vk.replace_text(2, 1, "Việt").unwrap();
    drop(vk);

    let (key_ops, mod_ops) = server_thread.join().unwrap();
    assert!(
        !mod_ops.is_empty() && mod_ops.iter().all(|&(_, locked)| (locked & XKB_MOD_MOD2) != 0),
        "NumLock (XKB_MOD_MOD2) must never be cleared during virtual-keyboard modifier updates: {mod_ops:?}"
    );
    assert_eq!(
        mod_ops.last().copied(),
        Some((0, XKB_MOD_LOCK | XKB_MOD_MOD2)),
        "CapsLock and NumLock must both be restored after atomic replacement"
    );
    let v_stroke = char_to_vk_stroke('V').unwrap();
    let i_stroke = char_to_vk_stroke('i').unwrap();
    let e_stroke = char_to_vk_stroke('ệ').unwrap();
    let t_stroke = char_to_vk_stroke('t').unwrap();

    assert_eq!(
        key_ops,
        vec![
            (30, 1),
            (30, 0),
            (KEY_DELETE, 1),
            (KEY_DELETE, 0),
            (KEY_BACKSPACE, 1),
            (KEY_BACKSPACE, 0),
            (KEY_BACKSPACE, 1),
            (KEY_BACKSPACE, 0),
            (v_stroke.evdev_code, 1),
            (v_stroke.evdev_code, 0),
            (i_stroke.evdev_code, 1),
            (i_stroke.evdev_code, 0),
            (e_stroke.evdev_code, 1),
            (e_stroke.evdev_code, 0),
            (t_stroke.evdev_code, 1),
            (t_stroke.evdev_code, 0),
        ]
    );
}

#[test]
fn dual_shift_handoff_and_global_remove_preserve_invariants() {
    let (client, mut server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    server
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    let server_thread = thread::spawn(move || {
        let (_, _, reg_args) = read_msg(&mut server);
        let registry_id = u32_ne(&reg_args);
        let (_, _, sync1_args) = read_msg(&mut server);
        let sync1_id = u32_ne(&sync1_args);

        let mut seat_global = 10u32.to_ne_bytes().to_vec();
        seat_global.extend(wl_string("wl_seat"));
        seat_global.extend(7u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &seat_global);

        let mut vk_global = 20u32.to_ne_bytes().to_vec();
        vk_global.extend(wl_string("zwp_virtual_keyboard_manager_v1"));
        vk_global.extend(1u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &vk_global);

        send_event(&mut server, sync1_id, 0, &1u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync1_id.to_ne_bytes());

        let _ = read_msg(&mut server); // bind seat
        let _ = read_msg(&mut server); // bind vk mgr
        let (_, _, create_vk_args) = read_msg(&mut server);
        let vk_id = u32_ne(&create_vk_args[4..]);
        let _ = read_msg(&mut server); // keymap
        let _ = read_msg(&mut server); // modifiers
        let (_, _, sync2_args) = read_msg(&mut server);
        let sync2_id = u32_ne(&sync2_args);
        send_event(&mut server, sync2_id, 0, &2u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync2_id.to_ne_bytes());

        let mut timestamps = Vec::new();
        let mut sent_global_remove = false;
        loop {
            let (obj, op, args) = read_msg(&mut server);
            if obj == vk_id && op == 3 {
                break;
            }
            if obj == vk_id && op == 1 {
                let ts = u32_ne(&args[0..4]);
                let key = u32_ne(&args[4..]);
                let state = u32_ne(&args[8..]);
                timestamps.push((ts, key, state));
                // After adopt_shift_from_uinput sends both shifts (4 events total: 2 release + 2 press),
                // simulate compositor removing the wl_seat global (opcode 1 = global_remove).
                if timestamps.len() == 4 && !sent_global_remove {
                    sent_global_remove = true;
                    send_event(&mut server, registry_id, 1, &10u32.to_ne_bytes());
                }
            }
        }
        timestamps
    });

    let conn = Connection::from_socket(client).unwrap();
    let vk = WaylandVirtualKeyboard::from_connection(conn, false, false).unwrap();
    vk.adopt_shift_from_uinput(true, true);
    let handed = vk.handoff_shift_to_uinput();
    assert_eq!(handed, (true, true));
    // Give the server thread a brief moment to emit global_remove, then trigger a flush_and_drain.
    thread::sleep(Duration::from_millis(20));
    let _ = vk.send_key_event(30, 1, false);
    assert!(
        !vk.is_healthy(),
        "GlobalRemove of wl_seat must mark WaylandVirtualKeyboard unhealthy for reconnect"
    );
    drop(vk);

    let timestamps = server_thread.join().unwrap();
    assert!(timestamps.len() >= 4);
    for pair in timestamps.windows(2) {
        assert!(
            pair[1].0 > pair[0].0,
            "Dual-shift timestamps must be strictly monotonic: {timestamps:?}"
        );
    }
}

#[test]
fn numlock_restored_across_uinput_and_vk_keymap_transitions() {
    let (client, mut server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    server
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    let server_thread = thread::spawn(move || {
        let (_, _, reg_args) = read_msg(&mut server);
        let registry_id = u32_ne(&reg_args);
        let (_, _, sync1_args) = read_msg(&mut server);
        let sync1_id = u32_ne(&sync1_args);

        let mut seat_global = 10u32.to_ne_bytes().to_vec();
        seat_global.extend(wl_string("wl_seat"));
        seat_global.extend(7u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &seat_global);

        let mut vk_global = 20u32.to_ne_bytes().to_vec();
        vk_global.extend(wl_string("zwp_virtual_keyboard_manager_v1"));
        vk_global.extend(1u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &vk_global);

        send_event(&mut server, sync1_id, 0, &1u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync1_id.to_ne_bytes());

        let _ = read_msg(&mut server); // bind wl_seat
        let _ = read_msg(&mut server); // bind manager
        let (_, _, create_args) = read_msg(&mut server);
        let vk_id = u32_ne(&create_args[4..]);
        let _ = read_msg(&mut server); // keymap
        let _ = read_msg(&mut server); // initial modifiers
        let (_, _, sync2_args) = read_msg(&mut server);
        let sync2_id = u32_ne(&sync2_args);
        send_event(&mut server, sync2_id, 0, &2u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync2_id.to_ne_bytes());

        let mut events = Vec::new();
        loop {
            let (obj, op, args) = read_msg(&mut server);
            if obj == vk_id && op == 3 {
                break;
            }
            if obj == vk_id && op == 1 {
                let key = u32_ne(&args[4..]);
                let state = u32_ne(&args[8..]);
                events.push(("key", key, state));
            } else if obj == vk_id && op == 2 {
                let depressed = u32_ne(&args[0..4]);
                let locked = u32_ne(&args[8..12]);
                events.push(("mod", depressed, locked));
            }
        }
        events
    });

    let conn = Connection::from_socket(client).unwrap();
    let vk = WaylandVirtualKeyboard::from_connection(conn, false, true).unwrap();

    // 1. First VK key activates the virtual keymap on Smithay/COSMIC -> must re-apply NumLock after key.
    assert!(vk.send_key_event(30, 1, false));
    assert!(vk.send_key_event(30, 0, false));

    // 2. Simulate uinput activity (e.g., Alt+Tab or Ctrl+Space) swapping compositor back to system keymap,
    //    then releasing last modifier -> sync_locked_modifiers(false) restores NumLock.
    vk.mark_uinput_activity();
    vk.sync_locked_modifiers(false);

    // 3. Subsequent VK ASCII replacement (e.g., Space) re-activates VK keymap -> must restore NumLock again.
    vk.replace_text(0, 0, " ").unwrap();
    drop(vk);

    let events = server_thread.join().unwrap();
    let mod_events: Vec<(u32, u32)> = events
        .iter()
        .filter(|&&(kind, _, _)| kind == "mod")
        .map(|&(_, dep, lck)| (dep, lck))
        .collect();
    assert_eq!(
        mod_events.len(),
        3,
        "Expected 3 NumLock restore modifier updates (initial VK key, post-uinput sync, post-uinput replace_text): {events:?}"
    );
    assert!(
        mod_events.iter().all(|&(_, locked)| locked == XKB_MOD_MOD2),
        "All modifier updates must preserve XKB_MOD_MOD2 (NumLock): {mod_events:?}"
    );
    // The first NumLock restore must come AFTER the first key press (so it runs after Smithay's set_keymap_from_string).
    assert_eq!(events[0], ("key", 30, 1));
    assert_eq!(events[1], ("mod", 0, XKB_MOD_MOD2));
}

#[test]
fn dvorak_capslock_unlocks_eight_level_vietnamese_keys() {
    use crate::layout::KeyboardLayout;
    use crate::wayland_vk_keymap::{
        char_to_vk_stroke_for_layout, is_eight_level_evdev, is_layout_alpha_evdev,
    };

    // On Dvorak, AD01 (evdev 16) is apostrophe (non-alpha), yet hosts lowercase 'à' on DBOARD_EIGHT_LEVEL.
    let stroke = char_to_vk_stroke_for_layout(KeyboardLayout::Dvorak, 'à').unwrap();
    assert_eq!(stroke.evdev_code, 16);
    assert!(
        !is_layout_alpha_evdev(KeyboardLayout::Dvorak, stroke.evdev_code),
        "AD01 is punctuation on Dvorak"
    );
    assert!(
        is_eight_level_evdev(stroke.evdev_code),
        "AD01 must still be detected as Lock-affected via DBOARD_EIGHT_LEVEL"
    );
    // Backspace / Enter / Space are never DBOARD_EIGHT_LEVEL.
    assert!(!is_eight_level_evdev(14));
    assert!(!is_eight_level_evdev(28));
    assert!(!is_eight_level_evdev(57));
}

#[test]
fn stalled_compositor_roundtrip_times_out_instead_of_hanging_evdev_loop() {
    let (client, _stalled_server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    let conn = Connection::from_socket(client).unwrap();
    let start = Instant::now();
    let err = match WaylandVirtualKeyboard::from_connection(conn, false, false) {
        Ok(_) => panic!("stalled compositor without sync reply must time out"),
        Err(err) => err,
    };
    let elapsed = start.elapsed();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        elapsed < Duration::from_secs(1),
        "bounded roundtrip took too long: {elapsed:?}"
    );
}

#[test]
fn disconnected_compositor_roundtrip_fails_immediately_without_spinning() {
    let (client, server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    drop(server);
    let conn = Connection::from_socket(client).unwrap();
    let start = Instant::now();
    let err = match WaylandVirtualKeyboard::from_connection(conn, false, false) {
        Ok(_) => panic!("disconnected compositor must fail immediately"),
        Err(err) => err,
    };
    assert_ne!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "disconnected roundtrip must not spin until timeout: {:?}",
        start.elapsed()
    );
}

#[test]
fn capslock_modifier_is_emitted_before_first_vk_key_after_uinput_activity() {
    let (client, mut server) = UnixStream::pair().unwrap();
    client.set_nonblocking(true).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    server
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();

    let server_thread = thread::spawn(move || {
        let (_, _, reg_args) = read_msg(&mut server);
        let registry_id = u32_ne(&reg_args);
        let (_, _, sync1_args) = read_msg(&mut server);
        let sync1_id = u32_ne(&sync1_args);

        let mut seat_global = 10u32.to_ne_bytes().to_vec();
        seat_global.extend(wl_string("wl_seat"));
        seat_global.extend(7u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &seat_global);

        let mut vk_global = 20u32.to_ne_bytes().to_vec();
        vk_global.extend(wl_string("zwp_virtual_keyboard_manager_v1"));
        vk_global.extend(1u32.to_ne_bytes());
        send_event(&mut server, registry_id, 0, &vk_global);

        send_event(&mut server, sync1_id, 0, &1u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync1_id.to_ne_bytes());

        let _ = read_msg(&mut server);
        let _ = read_msg(&mut server);
        let (_, _, create_args) = read_msg(&mut server);
        let vk_id = u32_ne(&create_args[4..]);
        let _ = read_msg(&mut server);
        let _ = read_msg(&mut server);
        let (_, _, sync2_args) = read_msg(&mut server);
        let sync2_id = u32_ne(&sync2_args);
        send_event(&mut server, sync2_id, 0, &2u32.to_ne_bytes());
        send_event(&mut server, 1, 1, &sync2_id.to_ne_bytes());

        let mut events = Vec::new();
        loop {
            let (obj, op, args) = read_msg(&mut server);
            if obj == vk_id && op == 3 {
                break;
            }
            if obj == vk_id && op == 1 {
                events.push(("key", u32_ne(&args[4..]), u32_ne(&args[8..])));
            } else if obj == vk_id && op == 2 {
                events.push(("mod", u32_ne(&args[0..4]), u32_ne(&args[8..12])));
            }
        }
        events
    });

    let conn = Connection::from_socket(client).unwrap();
    let vk = WaylandVirtualKeyboard::from_connection(conn, true, false).unwrap();
    vk.mark_uinput_activity();
    assert!(vk.send_key_event(30, 1, false));
    assert!(vk.send_key_event(30, 0, false));
    drop(vk);

    let events = server_thread.join().unwrap();
    assert_eq!(events[0], ("mod", 0, XKB_MOD_LOCK));
    assert_eq!(events[1], ("key", 30, 1));
    assert_eq!(events[2], ("mod", 0, XKB_MOD_LOCK));
}
