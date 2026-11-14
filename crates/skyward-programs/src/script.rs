//! The init scripts a phone's own partitions carry, read as init reads
//! them: the programs it runs, each with the user, the groups, the
//! capabilities, the sockets and the environment it is given, and what it
//! does when something happens, which is where a program it does not start
//! by itself is started.

use anyhow::{bail, ensure, Context, Result};

/// A program init runs, as a `service` section describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    /// The name init knows it by.
    pub name: String,
    pub path: String,
    pub args: Vec<String>,
    /// The classes it is started and stopped with; `default` where the
    /// section names none.
    pub classes: Vec<String>,
    /// The user it runs as; root where the section names none.
    pub user: Option<String>,
    /// Its group and then the others it is in.
    pub groups: Vec<String>,
    /// What it keeps of what only root may do; none where the section has
    /// no such line, which leaves root all of it and any other user none.
    pub capabilities: Option<Vec<String>>,
    pub sockets: Vec<Socket>,
    /// What is set in its environment, each a name and its value.
    pub environment: Vec<(String, String)>,
    /// The services the section says it registers, each as it is
    /// registered: the interface and then which of them it is.
    pub interfaces: Vec<String>,
    /// It runs once, to its end, and is not started again.
    pub oneshot: bool,
    /// Its classes do not start it: something has to start it by name.
    pub disabled: bool,
    /// The section takes the place of an earlier one of the same name.
    pub overrides: bool,
    /// The script the section is in, as the phone's scripts would name it,
    /// where the script was read as one of a phone's.
    pub script: String,
}

/// A socket init makes for a program before it starts it, under
/// /dev/socket, and hands it already open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socket {
    pub name: String,
    pub kind: SocketKind,
    /// The peer's credentials come with what it sends.
    pub passcred: bool,
    /// Init has it listen before it hands it over.
    pub listen: bool,
    pub mode: u32,
    /// The user and the group the socket is given; root where the line
    /// names none.
    pub user: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    Stream,
    Dgram,
    Seqpacket,
}

/// What init does when all of `triggers` hold, as an `on` section says it:
/// each command a line of words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub triggers: Vec<String>,
    pub commands: Vec<Vec<String>>,
}

/// One script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    /// The scripts it has init read as well, as it names them.
    pub imports: Vec<String>,
    pub programs: Vec<Program>,
    pub actions: Vec<Action>,
}

/// The section a line belongs to.
enum Section {
    None,
    Program,
    Action,
}

/// Read a script. Of what a `service` section may say, the lines that do
/// not bear on how a recovery starts the program are passed over, as are
/// sections of kinds other than the three read here.
pub fn read(text: &str) -> Result<Script> {
    let mut script = Script::default();
    let mut section = Section::None;
    for (number, words) in lines(text)? {
        let Some(first) = words.first() else { continue };
        match first.as_str() {
            "import" => {
                ensure!(words.len() == 2, "line {number}: an import names one script");
                script.imports.push(words[1].clone());
                section = Section::None;
            }
            "service" => {
                ensure!(words.len() >= 3, "line {number}: a service has a name and a program");
                script.programs.push(Program { name: words[1].clone(), path: words[2].clone(), args: words[3..].to_vec(), ..Program::default() });
                section = Section::Program;
            }
            "on" => {
                script.actions.push(Action { triggers: triggers(&words[1..]).with_context(|| format!("line {number}"))?, commands: Vec::new() });
                section = Section::Action;
            }
            _ => match section {
                Section::Program => {
                    let program = script.programs.last_mut().expect("a service section has begun");
                    option(program, &words).with_context(|| format!("line {number}, of the service {}", program.name))?;
                }
                Section::Action => script.actions.last_mut().expect("an on section has begun").commands.push(words),
                Section::None => bail!("line {number}: {first} begins no section, and no section has begun"),
            },
        }
    }
    for program in &mut script.programs {
        if program.classes.is_empty() {
            program.classes.push("default".into());
        }
    }
    Ok(script)
}

/// The lines of a script that hold words, each with the number of the line
/// it begins on and its words, split as init splits them: at blanks; with
/// what is between double quotes kept as it is; with a `#` where a word
/// would begin leaving the rest of the line out; and with a backslash
/// taking the character after it as it is, or, before `n`, `r` or `t`, as
/// the character C writes so, or, before the end of the line, carrying the
/// line on in the next past the blanks that one begins with.
fn lines(text: &str) -> Result<Vec<(usize, Vec<String>)>> {
    let mut out = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word: Option<String> = None;
    let (mut number, mut began) = (1, 1);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                words.extend(word.take());
                if !words.is_empty() {
                    out.push((began, std::mem::take(&mut words)));
                }
                number += 1;
                began = number;
            }
            ' ' | '\t' | '\r' => words.extend(word.take()),
            '#' if word.is_none() => {
                while chars.next_if(|&c| c != '\n').is_some() {}
            }
            '"' => {
                let quoted = word.get_or_insert_with(String::new);
                loop {
                    match chars.next().with_context(|| format!("line {began}: a quote is not closed"))? {
                        '"' => break,
                        '\n' => {
                            number += 1;
                            quoted.push('\n');
                        }
                        c => quoted.push(c),
                    }
                }
            }
            '\\' => match chars.next().with_context(|| format!("line {began}: a backslash ends the script"))? {
                '\n' => {
                    number += 1;
                    while chars.next_if(|&c| c == ' ' || c == '\t').is_some() {}
                }
                '\r' if chars.peek() == Some(&'\n') => {
                    chars.next();
                    number += 1;
                    while chars.next_if(|&c| c == ' ' || c == '\t').is_some() {}
                }
                escaped => word.get_or_insert_with(String::new).push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                }),
            },
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word.take());
    if !words.is_empty() {
        out.push((began, words));
    }
    Ok(out)
}

/// The triggers of an `on` line, which `&&` stands between.
fn triggers(words: &[String]) -> Result<Vec<String>> {
    ensure!(words.len() % 2 == 1, "an on section names what it waits for, one trigger or several joined by &&");
    let mut out = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if index % 2 == 0 {
            ensure!(word != "&&", "a trigger is missing before &&");
            out.push(word.clone());
        } else {
            ensure!(word == "&&", "the triggers of an on section are joined by &&");
        }
    }
    Ok(out)
}

/// Take what one line of a `service` section says of the program.
fn option(program: &mut Program, words: &[String]) -> Result<()> {
    let (name, rest) = (words[0].as_str(), &words[1..]);
    match name {
        "class" => {
            ensure!(!rest.is_empty(), "class names at least one class");
            program.classes = rest.to_vec();
        }
        "user" => {
            ensure!(rest.len() == 1, "user names one user");
            program.user = Some(rest[0].clone());
        }
        "group" => {
            ensure!(!rest.is_empty(), "group names at least one group");
            program.groups = rest.to_vec();
        }
        "capabilities" => program.capabilities = Some(rest.to_vec()),
        "socket" => program.sockets.push(socket(rest)?),
        "setenv" => {
            ensure!(rest.len() == 2, "setenv gives a name and a value");
            program.environment.push((rest[0].clone(), rest[1].clone()));
        }
        "interface" => {
            ensure!(rest.len() == 2, "interface gives an interface and which of them it is");
            // A service of the newer kind is written as `aidl` and its whole
            // name; one of the older kind as its interface and its instance.
            program.interfaces.push(match rest[0].as_str() {
                "aidl" => rest[1].clone(),
                interface => format!("{interface}/{}", rest[1]),
            });
        }
        "oneshot" => program.oneshot = true,
        "disabled" => program.disabled = true,
        "override" => program.overrides = true,
        _ => {}
    }
    Ok(())
}

/// A socket, as its line gives it: its name, its kind with what init does
/// to it besides making it, each after a plus, its mode in octal, and then
/// its user and its group where it names them.
fn socket(words: &[String]) -> Result<Socket> {
    ensure!(words.len() >= 3, "socket gives a name, a kind and a mode");
    let mut said = words[1].split('+');
    let kind = match said.next() {
        Some("stream") => SocketKind::Stream,
        Some("dgram") => SocketKind::Dgram,
        Some("seqpacket") => SocketKind::Seqpacket,
        other => bail!("a socket of the kind {} is none init makes", other.unwrap_or_default()),
    };
    let (mut passcred, mut listen) = (false, false);
    for also in said {
        match also {
            "passcred" => passcred = true,
            "listen" => listen = true,
            other => bail!("init does nothing called {other} to a socket"),
        }
    }
    let mode = u32::from_str_radix(&words[2], 8).with_context(|| format!("the socket's mode {} is not an octal number", words[2]))?;
    Ok(Socket { name: words[0].clone(), kind, passcred, listen, mode, user: words.get(3).cloned(), group: words.get(4).cloned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn a_service_section_gives_the_program_and_how_it_runs() {
        let script = read(
            "# The key service.\n\
             service vendor.keymint-qti /vendor/bin/hw/android.hardware.security.keymint-service-qti\n\
             \x20   class early_hal\n\
             \x20   user system\n\
             \x20   group system drmrpc\n\
             \n\
             service vendor.weaver /odm/bin/hw/android.hardware.weaver-service --instance default\n\
             \x20   class hal late_start\n\
             \x20   user system\n\
             \x20   group system drmrpc\n\
             \x20   capabilities WAKE_ALARM\n\
             \x20   interface aidl android.hardware.weaver.IWeaver/default\n\
             \x20   disabled\n\
             \x20   oneshot\n",
        )
        .unwrap();
        let [keymint, weaver] = &script.programs[..] else { panic!("two services") };
        assert_eq!((keymint.name.as_str(), keymint.path.as_str()), ("vendor.keymint-qti", "/vendor/bin/hw/android.hardware.security.keymint-service-qti"));
        assert_eq!((keymint.user.as_deref(), &keymint.groups[..], &keymint.classes[..]), (Some("system"), &strings(&["system", "drmrpc"])[..], &strings(&["early_hal"])[..]));
        assert_eq!((keymint.capabilities.as_ref(), keymint.disabled, keymint.oneshot), (None, false, false), "a section says only what is not as init has it by itself");
        assert_eq!(weaver.args, ["--instance", "default"]);
        assert_eq!(weaver.classes, ["hal", "late_start"]);
        assert_eq!(weaver.capabilities.as_deref(), Some(&strings(&["WAKE_ALARM"])[..]));
        assert_eq!(weaver.interfaces, ["android.hardware.weaver.IWeaver/default"]);
        assert!(weaver.disabled && weaver.oneshot);
    }

    #[test]
    fn a_program_with_no_class_is_of_the_default_one_and_root_where_it_names_no_user() {
        let script = read("service vendor.qseecomd /vendor/bin/qseecomd\n").unwrap();
        assert_eq!(script.programs[0].classes, ["default"]);
        assert_eq!((script.programs[0].user.as_ref(), script.programs[0].groups.len()), (None, 0));
    }

    #[test]
    fn sockets_and_the_environment_are_what_init_gives_the_program() {
        let script = read(
            "service vendor.ssgtzd /vendor/bin/ssgtzd\n\
             \x20   socket ssgtzd stream 0660 system system\n\
             \x20   socket notify dgram+passcred 0666\n\
             \x20   socket taken seqpacket+listen+passcred 0600 root\n\
             \x20   setenv LD_PRELOAD \"/vendor/lib64/lib a.so\"\n",
        )
        .unwrap();
        let program = &script.programs[0];
        assert_eq!(
            program.sockets,
            [
                Socket { name: "ssgtzd".into(), kind: SocketKind::Stream, passcred: false, listen: false, mode: 0o660, user: Some("system".into()), group: Some("system".into()) },
                Socket { name: "notify".into(), kind: SocketKind::Dgram, passcred: true, listen: false, mode: 0o666, user: None, group: None },
                Socket { name: "taken".into(), kind: SocketKind::Seqpacket, passcred: true, listen: true, mode: 0o600, user: Some("root".into()), group: None },
            ]
        );
        assert_eq!(program.environment, [("LD_PRELOAD".to_string(), "/vendor/lib64/lib a.so".to_string())], "a quoted value keeps its blanks");
        assert!(read("service a /a\n    socket a pipe 0660\n").is_err(), "no such kind of socket");
        assert!(read("service a /a\n    socket a stream+nonblock 0660\n").is_err(), "nothing init does to a socket");
        assert!(read("service a /a\n    socket a stream rw\n").is_err(), "a mode that is no number");
    }

    #[test]
    fn a_service_of_the_older_kind_is_named_by_its_interface_and_its_instance() {
        let script = read("service a /a\n    interface android.hardware.gatekeeper@1.0::IGatekeeper default\n").unwrap();
        assert_eq!(script.programs[0].interfaces, ["android.hardware.gatekeeper@1.0::IGatekeeper/default"]);
    }

    #[test]
    fn an_on_section_gives_its_triggers_and_its_commands() {
        let script = read(
            "import /vendor/etc/init/hw/init.${ro.hardware}.rc\n\
             on property:ro.special_edition=true && property:sys.boot_completed=1\n\
             \x20   start vendor.weaver\n\
             \x20   write /sys/a \"1 2\"\n\
             on boot\n\
             \x20   # Nothing more.\n\
             \x20   chown system system /dev/a\n",
        )
        .unwrap();
        assert_eq!(script.imports, ["/vendor/etc/init/hw/init.${ro.hardware}.rc"]);
        let [first, boot] = &script.actions[..] else { panic!("two on sections") };
        assert_eq!(first.triggers, ["property:ro.special_edition=true", "property:sys.boot_completed=1"]);
        assert_eq!(first.commands, [strings(&["start", "vendor.weaver"]), strings(&["write", "/sys/a", "1 2"])]);
        assert_eq!((boot.triggers.clone(), boot.commands.clone()), (strings(&["boot"]), vec![strings(&["chown", "system", "system", "/dev/a"])]));
        assert!(read("on boot &&\n").is_err(), "a trigger is missing after &&");
        assert!(read("on\n").is_err(), "an on section that waits for nothing");
    }

    #[test]
    fn a_line_ending_in_a_backslash_goes_on_in_the_next() {
        let script = read("service a /a \\\n    --one \\\r\n    --two\n    user system\n").unwrap();
        assert_eq!(script.programs[0].args, ["--one", "--two"]);
        assert_eq!(script.programs[0].user.as_deref(), Some("system"));
    }

    #[test]
    fn a_hash_where_a_word_would_begin_leaves_the_rest_of_the_line_out() {
        let script = read(
            "service a /a#b # the program\n\
             \x20   setenv FLAG true # Any nonempty value counts as true\n\
             \x20   # user nobody \\\n\
             \x20   user system\n\
             \x20   setenv TEXT \"a # b \\n\"\n",
        )
        .unwrap();
        let program = &script.programs[0];
        assert_eq!((program.path.as_str(), program.args.len()), ("/a#b", 0), "inside a word it is a character like any other");
        assert_eq!(program.user.as_deref(), Some("system"), "a comment does not carry on in the next line");
        assert_eq!(program.environment, [("FLAG".to_string(), "true".to_string()), ("TEXT".to_string(), "a # b \\n".to_string())], "between quotes nothing is read but the quote that ends them");
    }

    #[test]
    fn a_line_outside_any_section_is_refused() {
        assert!(read("user system\n").is_err());
        assert!(read("service a\n").is_err(), "a service without its program");
        assert!(read("service a /a\n    user\n").is_err(), "a user line that names none");
        assert!(read("service a /a\n    setenv \"A\n").is_err(), "a quote left open");
    }
}
