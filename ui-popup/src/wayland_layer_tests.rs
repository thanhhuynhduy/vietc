// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn selects_matching_monitor_and_translates_global_caret_coordinates() {
    let outputs = vec![
        OutputInfo {
            global_name: 10,
            output: None,
            xdg_output: None,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            logical_x: None,
            logical_y: None,
            logical_width: None,
            logical_height: None,
            scale: 1,
            rotated_90: false,
        },
        OutputInfo {
            global_name: 20,
            output: None,
            xdg_output: None,
            x: 1920,
            y: 0,
            width: 3840,
            height: 2160,
            logical_x: Some(1920),
            logical_y: Some(0),
            logical_width: Some(1920),
            logical_height: Some(1080),
            scale: 2,
            rotated_90: false,
        },
        OutputInfo {
            global_name: 30,
            output: None,
            xdg_output: None,
            x: -1080,
            y: 0,
            width: 1920,
            height: 1080,
            logical_x: None,
            logical_y: None,
            logical_width: None,
            logical_height: None,
            scale: 1,
            rotated_90: true,
        },
        // Fractional 150% HiDPI display: physical 1920x1080, integer wl_output scale 2,
        // authoritative zxdg_output_v1 logical size 1280x720 at x=3840.
        OutputInfo {
            global_name: 40,
            output: None,
            xdg_output: None,
            x: 3840,
            y: 0,
            width: 1920,
            height: 1080,
            logical_x: Some(3840),
            logical_y: Some(0),
            logical_width: Some(1280),
            logical_height: Some(720),
            scale: 2,
            rotated_90: false,
        },
    ];

    let (out_name, lw, lh, scale, local) = select_output_and_local_caret(
        &outputs,
        Some(CaretRect {
            x: 2100,
            y: 300,
            h: 22,
        }),
    );
    assert_eq!(out_name, Some(20));
    assert_eq!((lw, lh, scale), (1920, 1080, 2));
    assert_eq!(
        local,
        Some(CaretRect {
            x: 180,
            y: 300,
            h: 22
        })
    );

    // Rotated 90-degree left monitor (negative x, swapped 1080x1920 logical bounds)
    let (rot_name, rlw, rlh, rscale, rlocal) = select_output_and_local_caret(
        &outputs,
        Some(CaretRect {
            x: -500,
            y: 1600,
            h: 20,
        }),
    );
    assert_eq!(rot_name, Some(30));
    assert_eq!((rlw, rlh, rscale), (1080, 1920, 1));
    assert_eq!(
        rlocal,
        Some(CaretRect {
            x: 580,
            y: 1600,
            h: 20
        })
    );

    // Fractional 150% monitor: caret at global (4900, 620) is local (1060, 620)
    // which would have fallen outside `1920 / 2 = 960` without zxdg_output_v1.
    let (frac_name, flw, flh, fscale, flocal) = select_output_and_local_caret(
        &outputs,
        Some(CaretRect {
            x: 4900,
            y: 620,
            h: 20,
        }),
    );
    assert_eq!(frac_name, Some(40));
    assert_eq!((flw, flh, fscale), (1280, 720, 2));
    assert_eq!(
        flocal,
        Some(CaretRect {
            x: 1060,
            y: 620,
            h: 20
        })
    );
}

#[test]
fn roundtrip_with_timeout_returns_timed_out_on_stalled_compositor() {
    use std::os::unix::net::UnixStream;
    let (client_sock, _stalled_server) = UnixStream::pair().unwrap();
    let conn = Connection::from_socket(client_sock).unwrap();
    let mut queue = conn.new_event_queue::<LayerState>();
    let mut state = LayerState::default();
    let start = Instant::now();
    let err = roundtrip_with_timeout(
        &conn,
        &mut queue,
        &mut state,
        Duration::from_millis(50),
        "stalled test",
    )
    .expect_err("stalled compositor socket must time out");
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "roundtrip timeout must return promptly"
    );
}
