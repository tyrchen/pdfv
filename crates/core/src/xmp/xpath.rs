//! Bounded `XPath` 1.0 syntax and core function checks for XMP values.

use sxd_xpath_nounsafe::Factory;

use crate::{ParseError, Result};

const MAX_BYTES: usize = 1024;
const MAX_OPERATORS: usize = 64;

#[derive(Debug)]
struct Parentheses<'a> {
    function: Option<&'a str>,
    argument_start: usize,
    commas: usize,
}

impl Parentheses<'_> {
    fn close(self, source: &str, end: usize, expression: &mut String) -> Option<()> {
        if let Some(name) = self.function {
            let args = source
                .get(self.argument_start..end)?
                .trim_matches(is_xml_space);
            let count = if args.is_empty() {
                0
            } else {
                self.commas.saturating_add(1)
            };
            if !valid_arity(name, count) {
                return None;
            }
            if self.commas != 0 {
                expression.push(')');
            }
        }
        Some(())
    }
}

#[derive(Debug)]
struct CheckedExpression {
    syntax: String,
    named_operators: usize,
}

pub(super) fn validate(value: &str) -> Result<bool> {
    if value.len() > MAX_BYTES {
        return Err(ParseError::LimitExceeded {
            limit: "max_xmp_xpath_bytes",
        }
        .into());
    }
    let mut quote = None;
    let mut operators = 0_usize;
    for byte in value.bytes() {
        if let Some(terminator) = quote {
            if byte == terminator {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if b"([=-+*/<>,|".contains(&byte) {
            operators = operators.saturating_add(1);
        }
    }
    if operators > MAX_OPERATORS {
        return Err(ParseError::LimitExceeded {
            limit: "max_xmp_xpath_operators",
        }
        .into());
    }
    let Some(expression) = checked_expression(value) else {
        return Ok(false);
    };
    if operators.saturating_add(expression.named_operators) > MAX_OPERATORS {
        return Err(ParseError::LimitExceeded {
            limit: "max_xmp_xpath_operators",
        }
        .into());
    }
    Ok(!value.trim_matches(is_xml_space).is_empty()
        && Factory::new().build(&expression.syntax).is_ok())
}

fn checked_expression(value: &str) -> Option<CheckedExpression> {
    let mut remaining = value;
    let mut expression = String::with_capacity(value.len());
    let mut groups = Vec::new();
    let mut can_end_operand = false;
    let mut needs_node_test = false;
    let mut named_operators = 0_usize;
    while let Some(first) = remaining.chars().next() {
        let rest = remaining.get(first.len_utf8()..)?;
        if matches!(first, '\'' | '"') {
            let (literal, tail) = take_literal(remaining)?;
            expression.push_str(literal);
            remaining = tail;
            can_end_operand = true;
            needs_node_test = false;
            continue;
        }
        if is_name_start(first) {
            let (name, tail) = take_name(remaining)?;
            let next = tail.trim_start_matches(is_xml_space);
            let named_operator = can_end_operand && matches!(name, "and" | "or" | "div" | "mod");
            expression.push_str(name);
            if let Some(step) = next.strip_prefix("::") {
                expression.push_str("::");
                remaining = step;
                can_end_operand = false;
                needs_node_test = true;
            } else if !named_operator && let Some(arguments) = next.strip_prefix('(') {
                expression.push('(');
                if matches!(name, "node" | "text" | "comment" | "processing-instruction") {
                    let (literal, tail) = node_test(name, arguments)?;
                    expression.push_str(literal);
                    expression.push(')');
                    remaining = tail;
                    can_end_operand = true;
                } else {
                    if needs_node_test {
                        return None;
                    }
                    groups.push(Parentheses {
                        function: Some(name),
                        argument_start: value.len().saturating_sub(arguments.len()),
                        commas: 0,
                    });
                    remaining = arguments;
                    can_end_operand = false;
                }
                needs_node_test = false;
            } else {
                remaining = tail;
                can_end_operand = !named_operator && !name.ends_with(':');
                needs_node_test = false;
                named_operators = named_operators.saturating_add(usize::from(named_operator));
            }
            continue;
        }

        let offset = value.len().saturating_sub(remaining.len());
        match first {
            '(' => groups.push(Parentheses {
                function: None,
                argument_start: value.len().saturating_sub(rest.len()),
                commas: 0,
            }),
            ')' => groups.pop()?.close(value, offset, &mut expression)?,
            ',' => {
                let group = groups.last_mut().filter(|group| group.function.is_some())?;
                // The compiler carries operator recognition across commas.
                // Group subsequent arguments to reset it before each operand.
                if group.commas != 0 {
                    expression.push(')');
                }
                expression.push_str(",(");
                group.commas = group.commas.saturating_add(1);
                remaining = rest;
                can_end_operand = false;
                needs_node_test = false;
                continue;
            }
            _ => {}
        }
        if !is_xml_space(first) {
            can_end_operand = first.is_ascii_digit()
                || matches!(first, '.' | ')' | ']')
                || (first == '*' && !can_end_operand);
            needs_node_test = matches!(first, '/' | '@');
        }
        expression.push(first);
        remaining = rest;
    }
    groups.is_empty().then_some(CheckedExpression {
        syntax: expression,
        named_operators,
    })
}

fn take_literal(value: &str) -> Option<(&str, &str)> {
    let delimiter = value.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let body = value.get(delimiter.len_utf8()..)?;
    let end = body
        .find(delimiter)?
        .saturating_add(delimiter.len_utf8().saturating_mul(2));
    Some((value.get(..end)?, value.get(end..)?))
}

fn take_name(value: &str) -> Option<(&str, &str)> {
    // The compiler validates XML names. Keep non-ASCII characters with the
    // QName candidate and separate the axis delimiter from single colons.
    let end = value
        .find(|c: char| !is_name_start(c) && !c.is_ascii_digit() && !matches!(c, '-' | '.' | ':'))
        .unwrap_or(value.len());
    let end = value.get(..end)?.find("::").unwrap_or(end);
    Some((value.get(..end)?, value.get(end..)?))
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn node_test<'a>(name: &str, args: &'a str) -> Option<(&'a str, &'a str)> {
    let args = args.trim_start_matches(is_xml_space);
    if let Some(tail) = args.strip_prefix(')') {
        return Some(("", tail));
    }
    if name != "processing-instruction" {
        return None;
    }
    let (literal, tail) = take_literal(args)?;
    Some((
        literal,
        tail.trim_start_matches(is_xml_space).strip_prefix(')')?,
    ))
}

fn valid_arity(name: &str, count: usize) -> bool {
    match name {
        "last" | "position" | "true" | "false" => count == 0,
        "local-name" | "namespace-uri" | "name" | "string" | "string-length"
        | "normalize-space" | "number" => count <= 1,
        "count" | "id" | "boolean" | "not" | "lang" | "sum" | "floor" | "ceiling" | "round" => {
            count == 1
        }
        "starts-with" | "contains" | "substring-before" | "substring-after" => count == 2,
        "substring" => matches!(count, 2 | 3),
        "translate" => count == 3,
        "concat" => count >= 2,
        // Namespaced functions belong to the context that will evaluate the XPath.
        _ => name
            .split_once(':')
            .is_some_and(|(_, local)| !local.is_empty() && !local.contains(':')),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::validate;
    use crate::{ParseError, PdfvError, Result};

    #[rstest]
    #[case("//item[@value = '1']", true)]
    #[case("count  ( //item )", true)]
    #[case("contains(*,*)", true)]
    #[case("concat('a,b', substring('test', 1, 2), normalize-space())", true)]
    #[case("1 div (count(//item) + 1)", true)]
    #[case("child :: processing-instruction ( 'a,b' )", true)]
    #[case("p:fn(1, count(*))", true)]
    #[case("é:fn (1)", true)]
    #[case("'wrong() will not be called'", true)]
    #[case("child::div", true)]
    #[case("", false)]
    #[case("  \t\r\n", false)]
    #[case("[", false)]
    #[case("p:q:r()", false)]
    #[case("wrong()", false)]
    #[case("count()", false)]
    #[case("count(//x, //y)", false)]
    #[case("concat('only one, even with comma')", false)]
    #[case("p:fn(wrong())", false)]
    #[case("substring('test', 1, 2, 3)", false)]
    #[case("child::processing-instruction(1)", false)]
    #[case("child::node('name')", false)]
    #[case("child::count(node())", false)]
    #[case("//contains('a', 'b')", false)]
    fn test_should_validate_xpath_syntax_and_core_arity(
        #[case] expression: &str,
        #[case] expected: bool,
    ) -> Result<()> {
        assert_eq!(validate(expression)?, expected);
        Ok(())
    }

    #[test]
    fn test_should_keep_xpath_byte_and_operator_limits() -> Result<()> {
        assert!(validate(&format!("'{}'", "(".repeat(65)))?);
        assert!(matches!(
            validate(&format!("{}1", "-".repeat(65))),
            Err(PdfvError::Parse(ParseError::LimitExceeded {
                limit: "max_xmp_xpath_operators",
            }))
        ));
        assert!(matches!(
            validate(&format!("1{}", " div 1".repeat(65))),
            Err(PdfvError::Parse(ParseError::LimitExceeded {
                limit: "max_xmp_xpath_operators",
            }))
        ));
        assert!(matches!(
            validate(&"a".repeat(1025)),
            Err(PdfvError::Parse(ParseError::LimitExceeded {
                limit: "max_xmp_xpath_bytes",
            }))
        ));
        Ok(())
    }
}
