#!/usr/bin/env python3
"""Headless regression checks for installer security and pure-evdev invariants."""
import pathlib
import re
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class DevicePermissionsTests(unittest.TestCase):
    def test_installer_uses_session_access_without_world_permissions(self):
        for rel in ("install.sh", "packaging/70-dboard.rules"):
            with self.subTest(file=rel):
                source = (ROOT / rel).read_text()
                rules = re.findall(r'^(?:KERNEL|SUBSYSTEM)==.*$', source, re.MULTILINE)
                self.assertTrue(rules)
                for rule in rules:
                    self.assertIn('TAG+="uaccess"', rule)
                    self.assertNotIn('MODE="0666"', rule)
                    self.assertNotIn('MODE="0664"', rule)
                    self.assertNotIn('MODE="0644"', rule)
                    if 'KERNEL=="uinput"' in rule:
                        self.assertIn('MODE="0600"', rule)
                        self.assertIn('GROUP="root"', rule)
                    else:
                        # Must not clobber distro GROUP="input" / MODE="0660" on event* devices,
                        # and must scope uaccess to keyboard/pointer classes only.
                        self.assertNotIn('GROUP="root"', rule)
                        self.assertNotIn('MODE="0600"', rule)
                        self.assertIn('ENV{ID_INPUT_', rule)
        self.assertIn('/etc/udev/rules.d/70-dboard.rules', (ROOT / "install.sh").read_text())

    def test_upgrade_and_uninstall_revoke_existing_world_access(self):
        for name in ("install.sh", "uninstall.sh"):
            with self.subTest(script=name):
                source = (ROOT / name).read_text()
                if name == "install.sh":
                    self.assertIn("chmod o-rw /dev/uinput /dev/input/event*", source)
                else:
                    self.assertIn("for device in /dev/uinput /dev/input/event*;", source)
                    self.assertIn('chmod o-rw -- "$device"', source)
                self.assertIn("udevadm trigger --action=change --subsystem-match=input", source)
                self.assertIn("udevadm trigger --action=change --subsystem-match=misc", source)
        self.assertIn("/etc/udev/rules.d/70-dboard.rules", (ROOT / "uninstall.sh").read_text())


class InstallerPathTests(unittest.TestCase):
    def test_daemon_is_not_granted_privileged_capabilities(self):
        source = (ROOT / "install.sh").read_text()
        self.assertNotIn("cap_sys_admin", source)
        self.assertNotIn("cap_dac_override", source)
        self.assertNotRegex(source, r"setcap\s+[^-\n].*dboard-daemon")
        self.assertIn("setcap -r /usr/bin/dboard-daemon", source)

    def test_scripts_do_not_redownload_mutable_branches_as_root(self):
        for name in ("install.sh", "uninstall.sh"):
            with self.subTest(script=name):
                source = (ROOT / name).read_text()
                self.assertNotRegex(source, r"raw\.githubusercontent\.com/.*/main/.*\|\s*sudo\s+bash")
                self.assertIn(
                    "Refusing to re-download and execute a mutable remote script as root",
                    source,
                )

    def test_privileged_path_never_includes_user_cargo_directory(self):
        source = (ROOT / "install.sh").read_text()
        exports = re.findall(r"^\s*export PATH=.*$", source, re.MULTILINE)
        self.assertEqual(
            [line.strip() for line in exports],
            ['export PATH="/usr/sbin:/usr/bin:/sbin:/bin"'],
        )
        self.assertIn('env PATH="$CARGO_HOME/bin:$PATH"', source)

    def test_installer_stops_systemd_service_with_initialized_user_and_uid(self):
        source = (ROOT / "install.sh").read_text()
        uid_init = source.find('U_UID="$(id -u "$INSTALLING_USER"')
        stop_pos = source.find("systemctl --user stop dboard.service")
        self.assertGreater(uid_init, 0, "U_UID must be resolved from INSTALLING_USER")
        self.assertGreater(stop_pos, uid_init, "U_UID must be initialized before stopping dboard.service")
        stop_block = source[stop_pos - 260 : stop_pos + 80]
        self.assertIn("$INSTALLING_USER", stop_block)
        self.assertNotIn("$REAL_USER", stop_block)

    def test_uninstaller_cleans_popup_lock_file(self):
        source = (ROOT / "uninstall.sh").read_text()
        self.assertIn('"/run/user/$U_UID/dboard-popup.lock"', source)


class StartupFileTests(unittest.TestCase):
    def test_installer_does_not_modify_shell_startup_files_as_root(self):
        source = (ROOT / "install.sh").read_text()
        self.assertNotIn("for _rc in", source)
        self.assertNotIn('"$REAL_HOME/.config/environment.d"', source)
        self.assertNotIn('chown "$REAL_USER":"$REAL_USER" "$_rc"', source)


def extracted_function(script, name):
    source = (ROOT / script).read_text()
    function = re.search(r"^" + name + r"\(\) \{\n.*?^\}", source, re.MULTILINE | re.DOTALL)
    if function is None:
        raise AssertionError(f"missing helper: {name}")
    return function.group()


class AclCleanupTests(unittest.TestCase):
    def test_cleanup_uses_only_supplied_paths_and_reports_failures(self):
        helper = extracted_function("uninstall.sh", "clear_device_acls")
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "device"
            path.touch()
            for status in (0, 1):
                with self.subTest(status=status):
                    program = (
                        helper
                        + '\nsetfacl() { printf "%s\\n" "$@"; return '
                        + str(status)
                        + '; }\nclear_device_acls "$1" "$1-missing"\n'
                    )
                    result = subprocess.run(
                        ["bash", "-euc", program, "acl-test", str(path)],
                        text=True,
                        capture_output=True,
                        check=True,
                    )
                    self.assertEqual(result.stdout.splitlines(), ["-b", "--", str(path)])
                    if status:
                        self.assertIn("WARNING", result.stderr)
                        self.assertIn(str(path), result.stderr)
                    else:
                        self.assertEqual(result.stderr, "")

    def test_missing_setfacl_is_reported_without_aborting_uninstall(self):
        helper = extracted_function("uninstall.sh", "clear_device_acls")
        program = helper + "\ncommand() { return 1; }\nclear_device_acls\n"
        result = subprocess.run(
            ["bash", "-euc", program],
            text=True,
            capture_output=True,
            check=True,
        )
        self.assertIn("WARNING", result.stderr)
        self.assertIn("setfacl", result.stderr)

    def test_distribution_policy_is_reapplied_after_acl_cleanup(self):
        source = (ROOT / "uninstall.sh").read_text()
        cleanup = source.index("clear_device_acls /dev/uinput /dev/input/event*")
        for operation in (
            "control --reload-rules",
            "trigger --action=change --subsystem-match=input",
            "trigger --action=change --subsystem-match=misc",
            "settle",
        ):
            self.assertGreater(source.index("udevadm " + operation), cleanup)


class PureEvdevAndUpdaterSecurityTests(unittest.TestCase):
    def test_installer_and_deb_packager_have_zero_ibus_fcitx5_or_input_group(self):
        for rel in ("install.sh", "packaging/deb/build-deb.sh", "dboard.service", "dboard.toml"):
            with self.subTest(file=rel):
                lower = (ROOT / rel).read_text().lower()
                self.assertNotIn("ibus", lower)
                self.assertNotIn("fcitx", lower)
                self.assertNotRegex(lower, r"usermod\s+-ag\s+input")
                self.assertNotRegex(lower, r"adduser\s+.*\s+input")

    def test_deb_postinst_is_non_interactive_and_uses_70_udev_rules(self):
        deb_script = (ROOT / "packaging/deb/build-deb.sh").read_text()
        self.assertNotIn("zenity", deb_script)
        self.assertIn("packaging/70-dboard.rules", deb_script)
        self.assertNotIn("packaging/99-dboard.rules", deb_script)

    def test_tray_has_no_remote_binary_updater(self):
        tray_source = (ROOT / "ui/src/tray.rs").read_text()
        self.assertNotIn("khoavo.myds.me", tray_source)
        self.assertNotIn("curl", tray_source)
        self.assertNotIn("APPIMAGE", tray_source)

    def test_installer_preserves_non_empty_wayland_display_for_systemd_user(self):
        installer = (ROOT / "install.sh").read_text()
        self.assertNotIn('WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}" \\\n', installer)
        self.assertIn('"/run/user/$U_UID"/wayland-[0-9]*', installer)
        self.assertIn("systemctl --user restart dboard.service", installer)

    def test_offline_tarball_sets_bin_and_pkg_dirs_and_drops_privs_for_user_home(self):
        installer = (ROOT / "install.sh").read_text()
        self.assertRegex(
            installer,
            r'elif \[ -f "\./bin/dboard-daemon" \]; then\s+INSTALL_DIR="\$\(pwd\)"\s+BIN_DIR="\$INSTALL_DIR/bin"\s+PKG_DIR="\$INSTALL_DIR"',
        )
        self.assertIn("run_as_installing_user()", installer)
        self.assertNotIn('chown -R "$INSTALLING_USER:$INSTALLING_GROUP" "$USER_HOME/.config/dboard"', installer)
        self.assertNotIn('chown -R "$INSTALLING_USER:$INSTALLING_GROUP" "$USER_HOME/.config/systemd/user"', installer)


if __name__ == "__main__":
    unittest.main()
