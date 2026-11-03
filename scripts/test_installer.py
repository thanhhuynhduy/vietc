#!/usr/bin/env python3
"""Headless regression checks for installer security and config migrations."""
import pathlib
import re
import subprocess
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class DevicePermissionsTests(unittest.TestCase):
    def test_installer_uses_session_access_without_world_permissions(self):
        source = (ROOT / "install.sh").read_text()
        rules = re.findall(r'^(?:KERNEL|SUBSYSTEM)==.*$', source, re.MULTILINE)
        self.assertTrue(rules)
        for rule in rules:
            self.assertIn('MODE="0600"', rule)
            self.assertIn('GROUP="root"', rule)
            self.assertIn('TAG+="uaccess"', rule)
        # uaccess must be tagged before systemd's 73-seat-late.rules runs.
        self.assertIn('/etc/udev/rules.d/70-vietc.rules', source)

    def test_upgrade_and_uninstall_revoke_existing_world_access(self):
        for name in ("install.sh", "uninstall.sh"):
            with self.subTest(script=name):
                source = (ROOT / name).read_text()
                if name == 'install.sh':
                    self.assertIn('chmod o-rw /dev/uinput /dev/input/event*', source)
                else:
                    self.assertIn('for device in /dev/uinput /dev/input/event*;', source)
                    self.assertIn('chmod o-rw -- "$device"', source)
                self.assertIn('udevadm trigger --action=change --subsystem-match=input', source)
                self.assertIn('udevadm trigger --action=change --subsystem-match=misc', source)
        self.assertIn('/etc/udev/rules.d/70-vietc.rules', (ROOT / "uninstall.sh").read_text())


class InstallerPathTests(unittest.TestCase):
    def test_privileged_path_never_includes_user_cargo_directory(self):
        source = (ROOT / "install.sh").read_text()
        exports = re.findall(r'^\s*export PATH=.*$', source, re.MULTILINE)
        self.assertEqual([line.strip() for line in exports],
                         ['export PATH="/usr/sbin:/usr/bin:/sbin:/bin"'])
        self.assertIn('env PATH="$CARGO_HOME/bin:$PATH"', source)


class StartupFileTests(unittest.TestCase):
    def test_installer_does_not_modify_shell_startup_files_as_root(self):
        source = (ROOT / "install.sh").read_text()
        self.assertNotIn('for _rc in', source)
        self.assertNotIn('"$REAL_HOME/.config/environment.d"', source)
        self.assertNotIn('chown "$REAL_USER":"$REAL_USER" "$_rc"', source)


def extracted_function(script, name):
    source = (ROOT / script).read_text()
    function = re.search(r'^' + name + r'\(\) \{\n.*?^\}', source, re.MULTILINE | re.DOTALL)
    if function is None:
        raise AssertionError(f'missing helper: {name}')
    return function.group()


class AclCleanupTests(unittest.TestCase):
    def test_cleanup_uses_only_supplied_paths_and_reports_failures(self):
        helper = extracted_function('uninstall.sh', 'clear_device_acls')
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'device'
            path.touch()
            for status in (0, 1):
                with self.subTest(status=status):
                    program = helper + '\nsetfacl() { printf "%s\\n" "$@"; return ' + str(status) + '; }\nclear_device_acls "$1" "$1-missing"\n'
                    result = subprocess.run(['bash', '-euc', program, 'acl-test', str(path)],
                                            text=True, capture_output=True, check=True)
                    self.assertEqual(result.stdout.splitlines(), ['-b', '--', str(path)])
                    if status:
                        self.assertIn('WARNING', result.stderr)
                        self.assertIn(str(path), result.stderr)
                    else:
                        self.assertEqual(result.stderr, '')

    def test_missing_setfacl_is_reported_without_aborting_uninstall(self):
        helper = extracted_function('uninstall.sh', 'clear_device_acls')
        program = helper + '\ncommand() { return 1; }\nclear_device_acls\n'
        result = subprocess.run(['bash', '-euc', program], text=True,
                                capture_output=True, check=True)
        self.assertIn('WARNING', result.stderr)
        self.assertIn('setfacl', result.stderr)

    def test_distribution_policy_is_reapplied_after_acl_cleanup(self):
        source = (ROOT / 'uninstall.sh').read_text()
        cleanup = source.index('clear_device_acls /dev/uinput /dev/input/event*')
        for operation in ('control --reload-rules',
                          'trigger --action=change --subsystem-match=input',
                          'trigger --action=change --subsystem-match=misc', 'settle'):
            self.assertGreater(source.index('udevadm ' + operation), cleanup)


class TomlMigrationTests(unittest.TestCase):
    def migrate(self, original, force=False):
        helpers = extracted_function('install.sh', 'set_root_bool') + '\n' + extracted_function('install.sh', 'migrate_ibus_config')
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'config.toml'
            path.write_text(original)
            path.chmod(0o640)
            program = helpers + '\nmigrate_ibus_config "$1" "$2"\n'
            arguments = ['bash', '-euc', program, 'migration-test', str(path), str(force).lower()]
            subprocess.run(arguments, check=True)
            migrated = path.read_text()
            subprocess.run(arguments, check=True)
            self.assertEqual(path.read_text(), migrated, 'migration must be idempotent')
            self.assertEqual(path.stat().st_mode & 0o777, 0o640)
            self.assertEqual(list(path.parent.iterdir()), [path])
            return tomllib.loads(migrated)

    def test_misplaced_booleans_move_to_root_without_changing_values(self):
        for value in (False, True):
            with self.subTest(value=value):
                document = self.migrate('[macros]\nko = "không"\nauto_ibus = ' + str(value).lower() + '\nibus_engine = false # manual\n[other]\nkeep = true\n')
                self.assertIs(document['auto_ibus'], value)
                self.assertIs(document['ibus_engine'], False)
                self.assertEqual(document['macros'], {'ko': 'không'})
                self.assertEqual(document['other'], {'keep': True})

    def test_root_values_win_over_misplaced_booleans(self):
        document = self.migrate('auto_ibus = false\nibus_engine = true\n[macros]\nauto_ibus = true\nibus_engine = false\n')
        self.assertIs(document['auto_ibus'], False)
        self.assertIs(document['ibus_engine'], True)
        self.assertEqual(document['macros'], {})

    def test_same_named_string_macros_and_other_tables_remain_intact(self):
        document = self.migrate('[macros]\nauto_ibus = "automatic"\nibus_engine = "engine"\n[other]\nauto_ibus = false\nibus_engine = true\n')
        self.assertIs(document['auto_ibus'], True)
        self.assertIs(document['ibus_engine'], False)
        self.assertEqual(document['macros'], {'auto_ibus': 'automatic', 'ibus_engine': 'engine'})
        self.assertEqual(document['other'], {'auto_ibus': False, 'ibus_engine': True})

    def test_quoted_keys_comments_and_literal_string_macros_are_preserved(self):
        document = self.migrate('"auto_ibus" = false # manual\n["macros"]\n"auto_ibus" = \'automatic\'\n\'ibus_engine\' = false # legacy\n')
        self.assertIs(document['auto_ibus'], False)
        self.assertIs(document['ibus_engine'], False)
        self.assertEqual(document['macros'], {'auto_ibus': 'automatic'})

    def test_boolean_looking_lines_inside_multiline_macros_are_not_migrated(self):
        original = '[macros]\nauto_ibus = """text\nibus_engine = false\n[other]\n"""\nibus_engine = \'\'\'literal\nauto_ibus = false\n\'\'\'\n'
        document = self.migrate(original)
        self.assertIs(document['auto_ibus'], True)
        self.assertIs(document['ibus_engine'], False)
        self.assertEqual(document['macros'], tomllib.loads(original)['macros'])

    def test_force_ibus_overrides_only_ibus_engine(self):
        document = self.migrate('auto_ibus = false\n[macros]\nibus_engine = false\n', force=True)
        self.assertIs(document['auto_ibus'], False)
        self.assertIs(document['ibus_engine'], True)
        self.assertEqual(document['macros'], {})

    def test_defaults_are_inserted_before_tables(self):
        for original in ('', '# header\ninput_method = "telex"\n', '[macros]\nko = "không"\n'):
            with self.subTest(original=original):
                document = self.migrate(original)
                self.assertIs(document['auto_ibus'], True)
                self.assertIs(document['ibus_engine'], False)

    def test_existing_personal_config_uses_the_same_migration(self):
        source = (ROOT / 'install.sh').read_text()
        self.assertIn('migrate_ibus_config "$USER_HOME/.config/vietc/config.toml" "$FORCE_IBUS"', source)

    def test_root_flags_stay_outside_macros_and_update_idempotently(self):
        source = (ROOT / "install.sh").read_text()
        function = re.search(r'^set_root_bool\(\) \{\n.*?^\}', source, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(function, 'missing structural root-key migration')
        for original in [
            '[macros]\nko = "không"\n',
            'auto_ibus = false\n[macros]\nko = "không"\n',
            '# header\ninput_method = "telex"\n',
        ]:
            with self.subTest(original=original), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / 'config.toml'
                path.write_text(original)
                program = function.group() + '\nset_root_bool "$1" auto_ibus true\nset_root_bool "$1" ibus_engine true\n'
                for _ in range(2):
                    subprocess.run(['bash', '-euc', program, 'migration-test', str(path)], check=True)
                document = tomllib.loads(path.read_text())
                self.assertIs(document['auto_ibus'], True)
                self.assertIs(document['ibus_engine'], True)
                self.assertEqual(document.get('macros'), tomllib.loads(original).get('macros'))


if __name__ == "__main__":
    unittest.main()
