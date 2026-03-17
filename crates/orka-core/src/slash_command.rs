/// Parsed representation of a slash command like `/skill echo greeting=hello`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCommand {
    /// The command name (e.g. "skill").
    pub name: String,
    /// Positional and key=value arguments after the command name.
    pub args: Vec<String>,
    /// The original trimmed input string.
    pub raw: String,
}

/// Parse a slash command from user input.
///
/// Returns `Some(ParsedCommand)` if the input starts with `/` followed by a command name.
/// Returns `None` if the input is not a slash command.
pub fn parse_slash_command(input: &str) -> Option<ParsedCommand> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    if rest.is_empty() || rest.starts_with(' ') {
        return None;
    }

    let args = tokenize(rest);
    if args.is_empty() {
        return None;
    }

    Some(ParsedCommand {
        name: args[0].clone(),
        args: args[1..].to_vec(),
        raw: trimmed.to_string(),
    })
}

/// Simple tokenizer supporting quoted arguments.
/// `key=val` stays as one token; `"hello world"` is a single token with quotes stripped.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = '"';

    for ch in input.chars() {
        match ch {
            '"' | '\'' if !in_quote => {
                in_quote = true;
                quote_char = ch;
            }
            c if in_quote && c == quote_char => {
                in_quote = false;
            }
            ' ' | '\t' if !in_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_command() {
        let cmd = parse_slash_command("/help").unwrap();
        assert_eq!(cmd.name, "help");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn command_with_args() {
        let cmd = parse_slash_command("/skill echo greeting=hello").unwrap();
        assert_eq!(cmd.name, "skill");
        assert_eq!(cmd.args, vec!["echo", "greeting=hello"]);
    }

    #[test]
    fn quoted_args() {
        let cmd = parse_slash_command(r#"/skill echo message="hello world""#).unwrap();
        assert_eq!(cmd.name, "skill");
        assert_eq!(cmd.args, vec!["echo", "message=hello world"]);
    }

    #[test]
    fn single_quoted_args() {
        let cmd = parse_slash_command("/skill echo message='hello world'").unwrap();
        assert_eq!(cmd.args, vec!["echo", "message=hello world"]);
    }

    #[test]
    fn just_slash_returns_none() {
        assert!(parse_slash_command("/").is_none());
    }

    #[test]
    fn slash_with_space_returns_none() {
        assert!(parse_slash_command("/ help").is_none());
    }

    #[test]
    fn not_a_command() {
        assert!(parse_slash_command("hello world").is_none());
    }

    #[test]
    fn whitespace_trimmed() {
        let cmd = parse_slash_command("  /quit  ").unwrap();
        assert_eq!(cmd.name, "quit");
    }

    #[test]
    fn preserves_raw() {
        let cmd = parse_slash_command("  /skill echo  ").unwrap();
        assert_eq!(cmd.raw, "/skill echo");
    }

    #[test]
    fn empty_input() {
        assert!(parse_slash_command("").is_none());
    }
}
