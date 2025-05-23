use nu_ansi_term::Style as NuStyle;
use std::borrow::Cow;

use crate::core::config::QuotesConfig;
use crate::core::highlighter::Highlight;
use crate::core::highlighters::quote::State::{InsideQuote, OutsideQuote};
use crate::style::Style;

const RESET: &str = "\x1b[0m";

pub struct QuoteHighlighter {
    quotes_token: char,
    color: String,
}

impl QuoteHighlighter {
    pub fn new(config: QuotesConfig) -> Self {
        let color = ansi_color_code_without_reset(config.style);

        Self {
            quotes_token: config.quotes_token,
            color,
        }
    }
}

fn ansi_color_code_without_reset(style: Style) -> String {
    let nu_style = NuStyle::from(style);
    let styled_str = format!("{}", nu_style.paint(""));

    styled_str.replace(RESET, "")
}

impl Highlight for QuoteHighlighter {
    fn apply<'a>(&self, input: &'a str) -> Cow<'a, str> {
        let quotes_count = input
            .chars()
            .fold(0, |acc, ch| acc + (ch == self.quotes_token) as usize);

        if quotes_count == 0 || quotes_count % 2 != 0 {
            return Cow::Borrowed(input);
        }

        let mut state = OutsideQuote;
        let mut output = String::new();

        for ch in input.chars() {
            match &mut state {
                InsideQuote {
                    color_inside_quote: color,
                    potential_reset_code,
                } => {
                    if ch == self.quotes_token {
                        // End of a quoted segment: insert the closing quote and reset.
                        output.push(ch);
                        output.push_str(RESET);
                        state = OutsideQuote;
                        continue;
                    }

                    // Accumulate characters to see if we are matching a reset sequence.
                    potential_reset_code.push(ch);
                    if potential_reset_code.as_str() == RESET {
                        output.push_str(potential_reset_code);
                        output.push_str(color);
                        potential_reset_code.clear();
                    } else if !RESET.starts_with(potential_reset_code.as_str()) {
                        // The accumulated characters do not form the reset code.
                        output.push_str(potential_reset_code);
                        potential_reset_code.clear();
                    }
                }
                OutsideQuote => {
                    if ch == self.quotes_token {
                        // Start of a quoted segment: insert the color code and the quote.
                        output.push_str(&self.color);
                        output.push(ch);
                        state = InsideQuote {
                            color_inside_quote: self.color.clone(),
                            potential_reset_code: String::new(),
                        };
                        continue;
                    }
                    output.push(ch);
                }
            }
        }

        Cow::Owned(output)
    }
}

enum State {
    InsideQuote {
        color_inside_quote: String,
        potential_reset_code: String,
    },
    OutsideQuote,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tests::escape_code_converter::{ConvertEscapeCodes, ConvertHighlightCodes};
    use crate::style::{Color, Style};

    #[test]
    fn test_multiple() {
        let highlighter = QuoteHighlighter::new(QuotesConfig {
            quotes_token: '"',
            style: Style::new().fg(Color::Yellow),
        });

        let cases = vec![
            (
                r#"Lorem ipsum "dolor" sit amet"#,
                r#"Lorem ipsum [yellow]"dolor"[reset] sit amet"#,
            ),
            (
                r#"Lorem ipsum dolor sit amet, consectetur adipiscing elit"#,
                r#"Lorem ipsum dolor sit amet, consectetur adipiscing elit"#,
            ),
        ];

        for (input, expected) in cases {
            let actual = highlighter.apply(input);
            assert_eq!(expected, actual.to_string().convert_escape_codes());
        }
    }

    #[test]
    fn test_no_overwrite() {
        let highlighter = QuoteHighlighter::new(QuotesConfig {
            quotes_token: '"',
            style: Style::new().fg(Color::Yellow),
        });

        let input = r#"Hello "abc [red]def[reset] ghi" World"#.to_string().convert_highlight_codes();
        let expected = r#"Hello [yellow]"abc [red]def[reset][yellow] ghi"[reset] World"#;

        let actual = highlighter.apply(input.as_str());

        assert_eq!(actual.to_string().convert_escape_codes(), expected);
    }

    #[test]
    fn test_odd_number_of_highlight_tokens() {
        let highlighter = QuoteHighlighter::new(QuotesConfig {
            quotes_token: '"',
            style: Style::new().fg(Color::Yellow),
        });

        let input = r#"Hello "abc def ghi World"#;
        let expected = r#"Hello "abc def ghi World"#;

        let actual = highlighter.apply(input);

        assert_eq!(actual.to_string().convert_escape_codes(), expected);
    }
}
