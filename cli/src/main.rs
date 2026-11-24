use dboard_engine::{Engine, EngineEvent, EventStore, InputEvent, InputMethod};
use std::io::{self, BufRead, Write};

struct CliState {
    engine: Engine,
    method: InputMethod,
    events: EventStore,
    macros: Vec<(String, String)>,
    auto_restore: bool,
}

impl CliState {
    fn new() -> Self {
        Self {
            engine: Engine::new(InputMethod::Telex),
            method: InputMethod::Telex,
            events: EventStore::new(),
            macros: Vec::new(),
            auto_restore: true,
        }
    }

    fn set_method(&mut self, method: InputMethod) {
        self.method = method;
        self.engine.set_method(method);
    }

    fn status(&self) {
        println!("  Method: {:?}", self.method);
        println!("  Enabled: {}", self.engine.is_enabled());
        println!("  Auto-restore: {}", self.auto_restore);
        println!("  Buffer: {:?}", self.engine.buffer());
        println!("  Macros: {} defined", self.macros.len());
        for (s, e) in &self.macros {
            println!("    {} -> {}", s, e);
        }
        println!("  Events: {} recorded", self.events.len());
    }
}

fn run<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut state = CliState::new();

    print_help();

    loop {
        write!(writer, "> ")?;
        writer.flush()?;

        let mut input = String::new();
        if reader.read_line(&mut input)? == 0 {
            return Ok(());
        }
        let line = input.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "quit" || trimmed == "exit" {
            break;
        }

        if trimmed.starts_with(':') {
            handle_command(&mut state, trimmed);
            continue;
        }

        let (output, display, events) = simulate_line(&mut state, line);

        writeln!(writer, "  Events: {:?}", events)?;
        writeln!(writer, "  Raw: {:?}", output)?;
        writeln!(writer, "  Screen: {}", display)?;
    }

    Ok(())
}

fn simulate_line(state: &mut CliState, input: &str) -> (String, String, Vec<(char, EngineEvent)>) {
    state.engine.reset();

    let mut output = String::new();
    let mut events = Vec::new();
    let mut word_screen_len: usize = 0;

    for ch in input.chars() {
        state.events.push(InputEvent::KeyTyped(ch));

        match state.engine.process_key(ch) {
            None => {
                output.push(ch);
                if ch == '\x08' {
                    word_screen_len = word_screen_len.saturating_sub(1);
                } else if dboard_engine::is_flush_char(ch) {
                    word_screen_len = 0;
                } else {
                    word_screen_len += 1;
                }
            }
            Some(event) => {
                events.push((ch, event.clone()));
                match &event {
                    EngineEvent::Insert(text) | EngineEvent::Flush(text) => {
                        output.push_str(text);
                        word_screen_len = 0;
                    }
                    EngineEvent::Paste(text) => {
                        output.push_str(text);
                        word_screen_len = 0;
                    }
                    EngineEvent::Replace(replacement) => {
                        for _ in 0..replacement.delete_before_cursor {
                            output.push('\x08');
                        }
                        output.push_str(&replacement.insert);
                        if dboard_engine::is_flush_char(ch) {
                            output.push(ch);
                            word_screen_len = 0;
                        } else {
                            word_screen_len = word_screen_len
                                .saturating_sub(replacement.delete_before_cursor)
                                + replacement.insert.chars().count();
                        }
                    }
                    EngineEvent::UndoTones {
                        backspaces,
                        restored,
                    } => {
                        for _ in 0..*backspaces {
                            output.push('\x08');
                        }
                        output.push_str(restored);
                        word_screen_len =
                            word_screen_len.saturating_sub(*backspaces) + restored.chars().count();
                    }
                    EngineEvent::AutoRestore(word) => {
                        let del = if word_screen_len > 0 {
                            word_screen_len
                        } else {
                            word.chars().count()
                        };
                        for _ in 0..del {
                            output.push('\x08');
                        }
                        output.push_str(word);
                        if dboard_engine::is_flush_char(ch) {
                            output.push(ch);
                        }
                        word_screen_len = 0;
                    }
                }
            }
        }
    }

    if let Some(event) = state.engine.flush() {
        match &event {
            EngineEvent::Flush(text) | EngineEvent::Insert(text) => {
                output.push_str(text);
            }
            EngineEvent::Replace(replacement) => {
                for _ in 0..replacement.delete_before_cursor {
                    output.push('\x08');
                }
                output.push_str(&replacement.insert);
            }
            EngineEvent::AutoRestore(word) => {
                let del = if word_screen_len > 0 {
                    word_screen_len
                } else {
                    word.chars().count()
                };
                for _ in 0..del {
                    output.push('\x08');
                }
                output.push_str(word);
            }
            _ => {}
        }
        events.push(('\n', event));
    }

    let display = apply_backspaces(&output);
    (output, display, events)
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(&mut stdin.lock(), &mut stdout.lock())
}

fn print_help() {
    println!("Dboard IME - Test Harness");
    println!("=========================");
    println!("Type text with VNI/Telex to see engine output.");
    println!();
    println!("Commands:");
    println!("  :help              Show this help");
    println!("  :status            Show engine state");
    println!("  :vi                Enable Vietnamese mode");
    println!("  :en                Disable Vietnamese mode");
    println!("  :ar on|off         Toggle auto-restore");
    println!("  :ac on|off         Toggle autocorrect (Gboard SymSpell)");
    println!("  :cap on|off        Toggle auto-capitalize");
    println!("  :vni               Switch to VNI input");
    println!("  :telex             Switch to Telex input");
    println!("  :reset             Reset engine buffer");
    println!("  :buffer            Show composing buffer");
    println!("  :events            Show event store history");
    println!("  :events clear      Clear event store");
    println!("  :macros            List macros");
    println!("  :macro add <s> <e> Add macro shortcut->expansion");
    println!("  :macro rm <s>      Remove a macro");
    println!("  :macro clear       Clear all macros");
    println!("  quit/exit          Quit");
    println!();
}

fn handle_command(state: &mut CliState, input: &str) {
    let parts: Vec<&str> = input.splitn(4, ' ').collect();
    let cmd = parts[0];

    match cmd {
        ":help" | ":h" => print_help(),

        ":status" | ":st" => state.status(),

        ":vi" => {
            state.engine.set_enabled(true);
            println!("[Vietnamese mode ON]");
        }

        ":ac" => {
            if let Some(arg) = parts.get(1) {
                match *arg {
                    "on" => {
                        state.engine.set_autocorrect(true);
                        println!("[Autocorrect ON]");
                    }
                    "off" => {
                        state.engine.set_autocorrect(false);
                        println!("[Autocorrect OFF]");
                    }
                    _ => println!("Usage: :ac on|off"),
                }
            } else {
                println!("Usage: :ac on|off");
            }
        }

        ":cap" => {
            if let Some(arg) = parts.get(1) {
                match *arg {
                    "on" => {
                        state.engine.set_auto_capitalize(true);
                        println!("[Auto-capitalize ON]");
                    }
                    "off" => {
                        state.engine.set_auto_capitalize(false);
                        println!("[Auto-capitalize OFF]");
                    }
                    _ => println!("Usage: :cap on|off"),
                }
            } else {
                println!("Usage: :cap on|off");
            }
        }

        ":en" => {
            state.engine.set_enabled(false);
            println!("[Vietnamese mode OFF]");
        }

        ":ar" => {
            if parts.len() < 2 {
                println!("[Usage: :ar on|off]");
                return;
            }
            match parts[1] {
                "on" => {
                    state.auto_restore = true;
                    state.engine.set_auto_restore(true);
                    println!("[Auto-restore ON]");
                }
                "off" => {
                    state.auto_restore = false;
                    state.engine.set_auto_restore(false);
                    println!("[Auto-restore OFF]");
                }
                _ => println!("[Usage: :ar on|off]"),
            }
        }

        ":vni" => {
            state.set_method(InputMethod::Vni);
            println!("[Switched to VNI]");
        }

        ":telex" => {
            state.set_method(InputMethod::Telex);
            println!("[Switched to Telex]");
        }

        ":reset" => {
            state.engine.reset();
            println!("[Engine reset]");
        }

        ":buffer" => {
            println!("[Buffer: {:?}]", state.engine.buffer());
        }

        ":events" | ":ev" => {
            if parts.len() > 1 && parts[1] == "clear" {
                state.events.clear();
                println!("[Event store cleared]");
                return;
            }
            if state.events.is_empty() {
                println!("[No events]");
            } else {
                println!("[Events: {}]", state.events.len());
                for (i, event) in state.events.iter().enumerate() {
                    println!("  {}: {:?}", i, event);
                }
                println!("  Raw keystrokes: {:?}", state.events.raw_keystrokes());
                println!("  Pattern hash: {}", state.events.pattern_hash());
            }
        }

        ":macros" => {
            if state.macros.is_empty() {
                println!("[No macros defined]");
            } else {
                println!("[Macros: {}]", state.macros.len());
                for (s, e) in &state.macros {
                    println!("  {} -> {}", s, e);
                }
            }
        }

        ":macro" => {
            if parts.len() < 2 {
                println!(
                    "[Usage: :macro add <shortcut> <expansion> or :macro rm <s> or :macro clear]"
                );
                return;
            }
            match parts[1] {
                "add" | "a" => {
                    if parts.len() < 4 {
                        println!("[Usage: :macro add <shortcut> <expansion>]");
                        return;
                    }
                    let shortcut = parts[2].to_string();
                    let expansion = parts[3].to_string();
                    state.engine.add_macro(shortcut.clone(), expansion.clone());
                    if let Some(pos) = state.macros.iter().position(|(s, _)| *s == shortcut) {
                        state.macros[pos].1 = expansion.clone();
                    } else {
                        state.macros.push((shortcut.clone(), expansion.clone()));
                    }
                    println!("[Macro added: {} -> {}]", shortcut, expansion);
                }
                "rm" | "remove" | "del" => {
                    if parts.len() < 3 {
                        println!("[Usage: :macro rm <shortcut>]");
                        return;
                    }
                    let shortcut = parts[2];
                    if let Some(pos) = state.macros.iter().position(|(s, _)| s == shortcut) {
                        state.macros.remove(pos);
                        state.engine.clear_macros();
                        for (s, e) in &state.macros {
                            state.engine.add_macro(s.clone(), e.clone());
                        }
                        println!("[Macro removed: {}]", shortcut);
                    } else {
                        println!("[Macro not found: {}]", shortcut);
                    }
                }
                "clear" | "c" => {
                    state.engine.clear_macros();
                    state.macros.clear();
                    println!("[All macros cleared]");
                }
                _ => {
                    let shortcut = parts[1].to_string();
                    let expansion = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
                    if expansion.is_empty() {
                        println!("[Usage: :macro add <shortcut> <expansion>]");
                        return;
                    }
                    state.engine.add_macro(shortcut.clone(), expansion.clone());
                    if let Some(pos) = state.macros.iter().position(|(s, _)| *s == shortcut) {
                        state.macros[pos].1 = expansion.clone();
                    } else {
                        state.macros.push((shortcut.clone(), expansion.clone()));
                    }
                    println!("[Macro added: {} -> {}]", shortcut, expansion);
                }
            }
        }

        _ => {
            println!(
                "[Unknown command: {}. Type :help for available commands]",
                cmd
            );
        }
    }
}

fn apply_backspaces(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        if ch == '\x08' {
            result.pop();
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod phase4_tests {
    use super::*;
    use std::io::{Cursor, Error, ErrorKind};

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(Error::new(ErrorKind::BrokenPipe, "closed output"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(Error::new(ErrorKind::BrokenPipe, "closed output"))
        }
    }

    #[test]
    fn eof_exits_cleanly() {
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        assert!(run(&mut input, &mut output).is_ok());
    }

    #[test]
    fn quit_preserves_interactive_behavior() {
        let mut input = Cursor::new(b"quit\n".to_vec());
        let mut output = Vec::new();

        assert!(run(&mut input, &mut output).is_ok());
        assert_eq!(String::from_utf8(output).unwrap(), "> ");
    }

    #[test]
    fn broken_pipe_is_returned_instead_of_panicking() {
        let mut input = Cursor::new(b"quit\n".to_vec());
        let mut output = BrokenWriter;

        let error = run(&mut input, &mut output).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    }

    #[test]
    fn simulate_line_preserves_all_flush_delimiters_and_filenames() {
        let mut state = CliState::new();
        assert_eq!(
            simulate_line(&mut state, "raw_buf [test]").1,
            "raw_buf [test]"
        );
        assert_eq!(
            simulate_line(&mut state, "raw_buf [test] (nhuw) foo-bar/baz.py ").1,
            "raw_buf [test] (như) foo-bar/baz.py "
        );
        assert_eq!(
            simulate_line(&mut state, "mowr test.py xem list.txt suwar host.conf ").1,
            "mở test.py xem list.txt sửa host.conf "
        );
        assert_eq!(
            simulate_line(&mut state, "dOs cAs tOs add reddit hidden ").1,
            "dOs cAs tOs add reddit hidden "
        );
        // Trailing space flush + macro expansion
        state
            .engine
            .add_macro("ko".to_string(), "không".to_string());
        assert_eq!(simulate_line(&mut state, "ko cos ").1, "không có ");
    }
}
