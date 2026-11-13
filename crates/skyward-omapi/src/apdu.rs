//! The commands sent to a secure element and its answers, as Android's
//! secure element service checks the ones and goes on from the others: a
//! command's own length has to add up, a caller may not manage channels
//! itself, the channel a command goes down is written into its first byte,
//! and an answer that says there is more, or that the length asked for was
//! not the one, is followed up before the caller sees it.

/// The shortest command: its class, its instruction and its two
/// parameters.
const HEADER: usize = 4;

/// The most a command may ask to have answered.
const MOST_EXPECTED: usize = 65536;

/// The instructions a caller may not send down a channel it was given: the
/// one that opens and closes channels, and the one that selects an applet,
/// where it selects one by its name.
const MANAGE_CHANNEL: u8 = 0x70;
const SELECT: u8 = 0xa4;
const SELECT_BY_NAME: u8 = 0x04;

/// The instruction that fetches what is left of an answer.
const GET_RESPONSE: u8 = 0xc0;

/// What a secure element says first of how a command went: more of the
/// answer waits, as many bytes as the second byte says; and the length
/// asked for was wrong, the second byte the right one.
const MORE_WAITS: u8 = 0x61;
const WRONG_LENGTH: u8 = 0x6c;

/// The names an applet may be selected by are five to sixteen bytes long.
const NAME_BYTES: std::ops::RangeInclusive<usize> = 5..=16;

/// What a caller says, in the second parameter of the command that selects
/// an applet, it wants back of the applet: all the service takes.
const SELECT_ANSWERS: [u8; 4] = [0x00, 0x04, 0x08, 0x0c];

/// Why a command is not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// It is no command: what is wrong with it.
    Malformed(String),
    /// It is one a caller may not send: why.
    NotAllowed(&'static str),
}

/// Check that `command` is one a caller may send down a channel it was
/// given.
pub fn check(command: &[u8]) -> Result<(), Refused> {
    let malformed = |why: String| Err(Refused::Malformed(why));
    if command.len() < HEADER {
        return malformed(format!("a command of {} bytes has no room for its class, instruction and parameters", command.len()));
    }
    let (class, instruction) = (command[0], command[1]);
    if class == 0xff {
        return malformed("no command is of the class ff".into());
    }
    if matches!(instruction & 0xf0, 0x60 | 0x90) {
        return malformed(format!("no command has the instruction {instruction:02x}"));
    }
    lengths(command).or_else(malformed)?;
    // A class of the kind every secure element reads the same way, as
    // opposed to one an applet defines for itself.
    if class & 0x80 == 0 && class & 0x60 != 0x20 {
        if instruction == MANAGE_CHANNEL {
            return Err(Refused::NotAllowed("a caller does not open or close channels itself"));
        }
        if instruction == SELECT && command[2] == SELECT_BY_NAME {
            return Err(Refused::NotAllowed("a caller does not select another applet on a channel it was given"));
        }
    }
    Ok(())
}

/// Check that the lengths a command of more than its header gives add up
/// to its own: of the data it carries, in one byte or, after a zero, in
/// two, and of the answer it expects, the same way.
fn lengths(command: &[u8]) -> Result<(), String> {
    let expected = |most: usize| match most <= MOST_EXPECTED {
        true => Ok(()),
        false => Err(format!("a command cannot expect {most} bytes back")),
    };
    let rest = command.len() - HEADER;
    if rest == 0 {
        return Ok(());
    }
    if rest == 1 {
        return expected(usize::from(command[4]));
    }
    if command[4] != 0 {
        let data = usize::from(command[4]);
        return match rest {
            n if n == 1 + data => Ok(()),
            n if n == 2 + data => expected(usize::from(command[command.len() - 1])),
            _ => Err(format!("the command says it carries {data} bytes, and its own length says otherwise")),
        };
    }
    let two = |at: usize| usize::from(command[at]) << 8 | usize::from(command[at + 1]);
    if rest == 3 {
        return expected(two(5));
    }
    if rest < 3 {
        return Err("the command begins a length of two bytes and ends inside it".into());
    }
    let data = two(5);
    if data == 0 {
        return Err("the command says it carries no bytes, in the way that says how many".into());
    }
    match rest {
        n if n == 3 + data => Ok(()),
        n if n == 5 + data => expected(two(command.len() - 2)),
        _ => Err(format!("the command says it carries {data} bytes, and its own length says otherwise")),
    }
}

/// The class byte `class` with the channel numbered `channel` written into
/// it: the first four channels in its two low bits, the sixteen after them
/// in its four, in the form of the class that has room for them, which
/// keeps what the class said of the command being protected.
pub fn on_channel(class: u8, channel: u8) -> Option<u8> {
    match channel {
        0..=3 => Some(class & 0xbc | channel),
        4..=19 => {
            let protected = class & 0x40 == 0 && class & 0x0c != 0;
            let further = class & 0xb0 | 0x40 | (channel - 4);
            Some(if protected { further | 0x20 } else { further })
        }
        _ => None,
    }
}

/// What an answer calls for before the caller sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Nothing: it is the answer.
    Done,
    /// The command again, expecting the length the secure element named.
    Again(Vec<u8>),
    /// The command that fetches what is left of the answer, which goes
    /// after what came so far.
    More(Vec<u8>),
}

/// What `answer` to `command` calls for; none where it is too short to say
/// how the command went.
pub fn next(command: &[u8], answer: &[u8]) -> Option<Next> {
    let [.., first, second] = *answer else { return None };
    Some(match first {
        WRONG_LENGTH => {
            let mut again = command.to_vec();
            *again.last_mut()? = second;
            Next::Again(again)
        }
        MORE_WAITS => Next::More(vec![command[0], GET_RESPONSE, 0x00, 0x00, second]),
        _ => Next::Done,
    })
}

/// The answer so far with `more` of it joined on: what the first said of
/// how the command went gives way to what the rest says.
pub fn joined(so_far: &[u8], more: &[u8]) -> Vec<u8> {
    let mut all = so_far[..so_far.len().saturating_sub(2)].to_vec();
    all.extend_from_slice(more);
    all
}

/// The name a caller gives to open a channel on: none for an empty one,
/// which asks for the applet the secure element selects by itself; none of
/// the two where it is of no length a name may have.
pub fn name(given: &[u8]) -> Result<Option<&[u8]>, Refused> {
    match given.len() {
        0 => Ok(None),
        n if NAME_BYTES.contains(&n) => Ok(Some(given)),
        n => Err(Refused::Malformed(format!("an applet's name is 5 to 16 bytes long, not {n}"))),
    }
}

/// Whether the service takes `answers` as what a caller wants back of the
/// applet it selects.
pub fn answers_taken(answers: u8) -> bool {
    SELECT_ANSWERS.contains(&answers)
}

/// The command that selects the applet named `name`, on the channel
/// numbered `channel`: the first that goes by the name, or, `following`,
/// the one after the one selected now.
pub fn select(name: &[u8], channel: u8, following: bool) -> Option<Vec<u8>> {
    let mut command = vec![on_channel(0x00, channel)?, SELECT, SELECT_BY_NAME, if following { 0x02 } else { 0x00 }, name.len() as u8];
    command.extend_from_slice(name);
    Some(command)
}

/// How selecting the applet after the one selected now went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// Another applet goes by the name, and is selected.
    Another,
    /// None does.
    NoOther,
    /// The secure element said neither.
    Neither,
}

/// What the `answer` to [`select`] says.
pub fn selected(answer: &[u8]) -> Selected {
    let [.., first, second] = *answer else { return Selected::Neither };
    match u16::from(first) << 8 | u16::from(second) {
        status if status & 0xf000 == 0x9000 || status & 0xff00 == 0x6200 || status & 0xff00 == 0x6300 => Selected::Another,
        status if status & 0xff00 == 0x6a00 => Selected::NoOther,
        _ => Selected::Neither,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        digits.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
    }

    #[test]
    fn a_command_whose_lengths_add_up_is_taken_in_each_of_its_forms() {
        for command in [
            "80 ca 00 fe",
            "80 ca 00 fe 00",
            "80 20 00 00 02 31 32",
            "80 20 00 00 02 31 32 00",
            "80 ca 00 fe 00 01 00",
            "80 20 00 00 00 00 02 31 32",
            "80 20 00 00 00 00 02 31 32 01 00",
        ] {
            assert_eq!(check(&hex(command)), Ok(()), "{command}");
        }
    }

    #[test]
    fn a_command_whose_lengths_do_not_add_up_is_refused() {
        for command in ["80 ca 00", "", "80 20 00 00 03 31 32", "80 20 00 00 02 31 32 00 00", "80 ca 00 fe 00 01", "80 20 00 00 00 00 00 31", "80 20 00 00 00 00 02 31 32 01", "ff ca 00 fe", "80 6a 00 00", "80 90 00 00"] {
            assert!(matches!(check(&hex(command)), Err(Refused::Malformed(_))), "{command}");
        }
    }

    #[test]
    fn a_caller_manages_no_channels_and_selects_no_other_applet() {
        assert!(matches!(check(&hex("00 70 00 00 01")), Err(Refused::NotAllowed(_))), "open a channel");
        assert!(matches!(check(&hex("01 70 80 01")), Err(Refused::NotAllowed(_))), "close one");
        assert!(matches!(check(&hex("00 a4 04 00 05 a0 00 00 00 03")), Err(Refused::NotAllowed(_))), "select by name");
        assert_eq!(check(&hex("00 a4 00 0c 02 3f 00")), Ok(()), "select a file, which is not an applet");
        assert_eq!(check(&hex("80 70 00 00 01")), Ok(()), "an applet's own instruction that happens to share the number");
    }

    #[test]
    fn the_channel_is_written_into_the_class() {
        assert_eq!(on_channel(0x00, 0), Some(0x00));
        assert_eq!(on_channel(0x00, 1), Some(0x01));
        assert_eq!(on_channel(0x80, 3), Some(0x83));
        assert_eq!(on_channel(0x03, 1), Some(0x01), "the channel a caller wrote itself is written over");
        assert_eq!(on_channel(0x00, 4), Some(0x40));
        assert_eq!(on_channel(0x80, 19), Some(0xcf));
        assert_eq!(on_channel(0x04, 5), Some(0x61), "a protected command stays protected past the fourth channel");
        assert_eq!(on_channel(0x00, 20), None, "there are twenty channels");
    }

    #[test]
    fn an_answer_that_says_more_waits_is_followed_up() {
        let command = hex("81 ca 00 fe 00");
        assert_eq!(next(&command, &hex("01 02 90 00")), Some(Next::Done));
        assert_eq!(next(&command, &hex("6a 82")), Some(Next::Done), "a failure is an answer too");
        assert_eq!(next(&command, &hex("01 02 61 10")), Some(Next::More(hex("81 c0 00 00 10"))), "on the channel the command went down");
        assert_eq!(next(&command, &hex("6c 20")), Some(Next::Again(hex("81 ca 00 fe 20"))));
        assert_eq!(next(&command, &[0x90]), None, "too short to say how the command went");
        assert_eq!(next(&command, &[]), None);
        assert_eq!(joined(&hex("01 02 61 02"), &hex("03 04 90 00")), hex("01 02 03 04 90 00"));
        assert_eq!(joined(&hex("61 02"), &hex("03 04 90 00")), hex("03 04 90 00"));
    }

    #[test]
    fn an_applet_is_named_in_five_to_sixteen_bytes_or_not_at_all() {
        assert_eq!(name(&[]), Ok(None));
        assert_eq!(name(&[1, 2, 3, 4, 5]), Ok(Some(&[1u8, 2, 3, 4, 5][..])));
        assert!(name(&[0; 16]).is_ok());
        assert!(name(&[0; 4]).is_err());
        assert!(name(&[0; 17]).is_err());
        assert!(answers_taken(0x00) && answers_taken(0x0c));
        assert!(!answers_taken(0x01));
    }

    #[test]
    fn the_applet_after_the_one_selected_is_asked_for_by_the_same_name() {
        let name = hex("a0 00 00 00 03 00 00 00");
        assert_eq!(select(&name, 2, true), Some(hex("02 a4 04 02 08 a0 00 00 00 03 00 00 00")));
        assert_eq!(select(&name, 0, false), Some(hex("00 a4 04 00 08 a0 00 00 00 03 00 00 00")));
        assert_eq!(select(&name, 20, true), None);
        assert_eq!(selected(&hex("6f 00 90 00")), Selected::Another);
        assert_eq!(selected(&hex("62 83")), Selected::Another, "selected, with a warning");
        assert_eq!(selected(&hex("6a 82")), Selected::NoOther);
        assert_eq!(selected(&hex("6d 00")), Selected::Neither);
        assert_eq!(selected(&[0x90]), Selected::Neither);
    }
}
