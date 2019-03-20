use pest::iterators::Pair;
use pest::Parser;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use dhall_parser::{DhallParser, Rule};

use crate::core;
use crate::core::*;

// This file consumes the parse tree generated by pest and turns it into
// our own AST. All those custom macros should eventually moved into
// their own crate because they are quite general and useful. For now they
// are here and hopefully you can figure out how they work.

pub type ParsedExpr = Expr<X, Import>;
pub type ParsedText = InterpolatedText<X, Import>;
pub type ParsedTextContents<'a> = InterpolatedTextContents<'a, X, Import>;
pub type RcExpr = Rc<ParsedExpr>;

pub type ParseError = pest::error::Error<Rule>;

pub type ParseResult<T> = Result<T, ParseError>;

pub fn custom_parse_error(pair: &Pair<Rule>, msg: String) -> ParseError {
    let msg =
        format!("{} while matching on:\n{}", msg, debug_pair(pair.clone()));
    let e = pest::error::ErrorVariant::CustomError { message: msg };
    pest::error::Error::new_from_span(e, pair.as_span())
}

fn debug_pair(pair: Pair<Rule>) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    fn aux(s: &mut String, indent: usize, prefix: String, pair: Pair<Rule>) {
        let indent_str = "| ".repeat(indent);
        let rule = pair.as_rule();
        let contents = pair.as_str();
        let mut inner = pair.into_inner();
        let mut first = true;
        while let Some(p) = inner.next() {
            if first {
                first = false;
                let last = inner.peek().is_none();
                if last && p.as_str() == contents {
                    let prefix = format!("{}{:?} > ", prefix, rule);
                    aux(s, indent, prefix, p);
                    continue;
                } else {
                    writeln!(
                        s,
                        r#"{}{}{:?}: "{}""#,
                        indent_str, prefix, rule, contents
                    )
                    .unwrap();
                }
            }
            aux(s, indent + 1, "".into(), p);
        }
        if first {
            writeln!(
                s,
                r#"{}{}{:?}: "{}""#,
                indent_str, prefix, rule, contents
            )
            .unwrap();
        }
    }
    aux(&mut s, 0, "".into(), pair);
    s
}

macro_rules! match_pair {
    (@make_child_match, ($($vars:tt)*), ($($outer_acc:tt)*), ($($acc:tt)*), ($(,)* $ty:ident ($x:ident..) $($rest_of_match:tt)*) => $body:expr, $($rest:tt)*) => {
        match_pair!(@make_child_match, ($($vars)*), ($($outer_acc)*), ($($acc)*, x..), ($($rest_of_match)*) => {
            let $x = x.map(|x| x.$ty());
            $body
        }, $($rest)*)
    };
    (@make_child_match, ($($vars:tt)*), ($($outer_acc:tt)*), ($($acc:tt)*), ($(,)* $ty:ident ($x:pat)  $($rest_of_match:tt)*) => $body:expr, $($rest:tt)*) => {
        match_pair!(@make_child_match, ($($vars)*), ($($outer_acc)*), ($($acc)*, ParsedValue::$ty($x)), ($($rest_of_match)*) => $body, $($rest)*)
    };
    (@make_child_match, ($($vars:tt)*), ($($outer_acc:tt)*), (, $($acc:tt)*), ($(,)*) => $body:expr, $($rest:tt)*) => {
        match_pair!(@make_matches, ($($vars)*), ([$($acc)*] => { $body }, $($outer_acc)*), $($rest)*)
    };
    (@make_child_match, ($($vars:tt)*), ($($outer_acc:tt)*), (), ($(,)*) => $body:expr, $($rest:tt)*) => {
        match_pair!(@make_matches, ($($vars)*), ([] => { $body }, $($outer_acc)*), $($rest)*)
    };

    (@make_matches, ($($vars:tt)*), ($($acc:tt)*), [$($args:tt)*] => $body:expr, $($rest:tt)*) => {
        match_pair!(@make_child_match, ($($vars)*), ($($acc)*), (), ($($args)*) => $body, $($rest)*)
    };
    (@make_matches, ($pair:expr, $parsed:expr), ($($acc:tt)*) $(,)*) => {
        {
            let pair = $pair;
            let rule = pair.as_rule();
            #[allow(unreachable_code)]
            iter_patterns::match_vec!($parsed;
                $($acc)*
                [x..] => panic!("Unexpected children while parsing rule '{:?}': {:?}", rule, x.collect::<Vec<_>>()),
            ).ok_or_else(|| custom_parse_error(&pair, "No match found".to_owned()))
        }
    };

    (($($vars:tt)*); $( [$($args:tt)*] => $body:expr ),* $(,)*) => {
        match_pair!(@make_matches, ($($vars)*), (), $( [$($args)*] => $body ),* ,)
    };
}

macro_rules! make_parser {
    // Filter out definitions that should not be matched on (i.e. rule_group)
    (@filter, rule) => (true);
    (@filter, rule_in_group) => (true);
    (@filter, rule_group) => (false);

    (@body, $pair:expr, $parsed:expr, rule!( $name:ident<$o:ty>; $($args:tt)* )) => (
        make_parser!(@body, $pair, $parsed, rule_in_group!( $name<$o>; $name; $($args)* ))
    );
    (@body, $pair:expr, $parsed:expr, rule_in_group!( $name:ident<$o:ty>; $group:ident; raw_pair!($x:pat) => $body:expr )) => ( {
        let $x = $pair.clone();
        let res: $o = $body;
        Ok(ParsedValue::$group(res))
    });
    (@body, $pair:expr, $parsed:expr, rule_in_group!( $name:ident<$o:ty>; $group:ident; captured_str!($x:ident) => $body:expr )) => ( {
        let $x = $pair.as_str();
        let res: $o = $body;
        Ok(ParsedValue::$group(res))
    });
    (@body, $pair:expr, $parsed:expr, rule_in_group!( $name:ident<$o:ty>; $group:ident; children!( $($args:tt)* ) )) => ( {
        let res: $o = match_pair!(($pair, $parsed); $($args)*)?;
        Ok(ParsedValue::$group(res))
    });
    (@body, $pair:expr, $parsed:expr, rule_group!( $name:ident<$o:ty> )) => (
        unreachable!()
    );


    ($( $submac:ident!( $name:ident<$o:ty> $($args:tt)* ); )*) => (
        // #[allow(non_camel_case_types, dead_code)]
        // enum ParsedType {
        //     $( $name, )*
        // }

        // impl ParsedType {
        //     #[allow(dead_code)]
        //     fn parse(self, pair: Pair<Rule>) -> ParseResult<ParsedValue> {
        //         match self {
        //             $( ParsedType::$name => {
        //                 let ret = $name(pair)?;
        //                 Ok(ParsedValue::$name(ret))
        //             }, )*
        //         }
        //     }
        //     // fn parse(self, pair: Pair<Rule>) -> ParseResult<ParsedValue> {
        //     //     match self {
        //     //         $( ParsedType::$name => $name(pair), )*
        //     //     }
        //     // }
        // }

        #[allow(non_camel_case_types, dead_code)]
        #[derive(Debug)]
        enum ParsedValue<'a> {
            $( $name($o), )*
        }

        impl<'a> ParsedValue<'a> {
            $(
                #[allow(non_snake_case, dead_code)]
                fn $name(self) -> $o {
                    match self {
                        ParsedValue::$name(x) => x,
                        _ => unreachable!(),
                    }
                }
            )*
        }

        // Non-recursive implementation to avoid stack overflows
        fn parse_any<'a>(initial_pair: Pair<'a, Rule>) -> ParseResult<ParsedValue<'a>> {
            enum StackFrame<'a> {
                Unprocessed(Pair<'a, Rule>),
                Processed(Pair<'a, Rule>, usize),
            }
            use StackFrame::*;
            let mut pairs_stack: Vec<StackFrame> = vec![Unprocessed(initial_pair.clone())];
            let mut values_stack: Vec<ParsedValue> = vec![];
            while let Some(p) = pairs_stack.pop() {
                match p {
                    Unprocessed(pair) => {
                        let mut pairs: Vec<_> = pair.clone().into_inner().map(StackFrame::Unprocessed).collect();
                        let n_children = pairs.len();
                        pairs_stack.push(Processed(pair, n_children));
                        pairs_stack.append(&mut pairs);
                    }
                    Processed(pair, n) => {
                        let mut parsed: Vec<_> = values_stack.split_off(values_stack.len() - n);
                        parsed.reverse();
                        let val = match pair.as_rule() {
                            $(
                                Rule::$name if make_parser!(@filter, $submac)
                                =>
                                make_parser!(@body, pair, parsed, $submac!( $name<$o> $($args)* ))
                                ,
                            )*
                            r => Err(custom_parse_error(&pair, format!("parse_any: Unexpected {:?}", r))),
                        }?;
                        values_stack.push(val);
                    }
                }
            }
            Ok(values_stack.pop().unwrap())
        }
    );
}

// List of rules that can be shortcutted if they have a single child
fn can_be_shortcutted(rule: Rule) -> bool {
    use Rule::*;
    match rule {
        import_alt_expression
        | or_expression
        | plus_expression
        | text_append_expression
        | list_append_expression
        | and_expression
        | combine_expression
        | prefer_expression
        | combine_types_expression
        | times_expression
        | equal_expression
        | not_equal_expression
        | application_expression
        | selector_expression_raw
        | annotated_expression => true,
        _ => false,
    }
}

make_parser! {
rule!(EOI<()>; raw_pair!(_) => ());

rule!(label_raw<Label>; captured_str!(s) => Label::from(s.trim().to_owned()));

rule!(double_quote_literal<ParsedText>; children!(
    [double_quote_chunk(chunks..)] => {
        chunks.collect()
    }
));

rule!(double_quote_chunk<ParsedTextContents<'a>>; children!(
    [interpolation(e)] => {
        InterpolatedTextContents::Expr(e)
    },
    [double_quote_escaped(s)] => {
        InterpolatedTextContents::Text(s)
    },
    [double_quote_char(s)] => {
        InterpolatedTextContents::Text(s)
    },
));
rule!(double_quote_escaped<&'a str>;
    // TODO: parse all escapes
    captured_str!(s) => {
        match s {
            "\"" => "\"",
            "$" => "$",
            "\\" => "\\",
            "/" => "/",
            // "b" => "\b",
            // "f" => "\f",
            "n" => "\n",
            "r" => "\r",
            "t" => "\t",
            // "uXXXX"
            _ => unimplemented!(),
        }
    }
);
rule!(double_quote_char<&'a str>;
    captured_str!(s) => s
);

rule!(end_of_line<()>; raw_pair!(_) => ());

rule!(single_quote_literal<ParsedText>; children!(
    [end_of_line(eol), single_quote_continue(contents)] => {
        contents.into_iter().rev().collect::<ParsedText>()
    }
));
rule!(single_quote_char<&'a str>;
    captured_str!(s) => s
);
rule!(escaped_quote_pair<&'a str>;
    raw_pair!(_) => "''"
);
rule!(escaped_interpolation<&'a str>;
    raw_pair!(_) => "${"
);
rule!(interpolation<RcExpr>; children!(
    [expression(e)] => e
));

rule!(single_quote_continue<Vec<ParsedTextContents<'a>>>; children!(
    [interpolation(c), single_quote_continue(rest)] => {
        let mut rest = rest;
        rest.push(InterpolatedTextContents::Expr(c)); rest
    },
    [escaped_quote_pair(c), single_quote_continue(rest)] => {
        let mut rest = rest;
        rest.push(InterpolatedTextContents::Text(c)); rest
    },
    [escaped_interpolation(c), single_quote_continue(rest)] => {
        let mut rest = rest;
        rest.push(InterpolatedTextContents::Text(c)); rest
    },
    [single_quote_char(c), single_quote_continue(rest)] => {
        let mut rest = rest;
        rest.push(InterpolatedTextContents::Text(c)); rest
    },
    [] => {
        vec![]
    },
));

rule!(NaN_raw<()>; raw_pair!(_) => ());
rule!(minus_infinity_literal<()>; raw_pair!(_) => ());
rule!(plus_infinity_literal<()>; raw_pair!(_) => ());

rule!(double_literal_raw<core::Double>;
    raw_pair!(pair) => {
        pair.as_str().trim()
            .parse()
            .map_err(|e: std::num::ParseFloatError| custom_parse_error(&pair, format!("{}", e)))?
    }
);

rule!(natural_literal_raw<core::Natural>;
    raw_pair!(pair) => {
        pair.as_str().trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| custom_parse_error(&pair, format!("{}", e)))?
    }
);

rule!(integer_literal_raw<core::Integer>;
    raw_pair!(pair) => {
        pair.as_str().trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| custom_parse_error(&pair, format!("{}", e)))?
    }
);

rule!(path<PathBuf>;
    captured_str!(s) => (".".to_owned() + s).into()
);

rule_group!(local_raw<(FilePrefix, PathBuf)>);

rule_in_group!(parent_path<(FilePrefix, PathBuf)>; local_raw; children!(
    [path(p)] => (FilePrefix::Parent, p)
));

rule_in_group!(here_path<(FilePrefix, PathBuf)>; local_raw; children!(
    [path(p)] => (FilePrefix::Here, p)
));

rule_in_group!(home_path<(FilePrefix, PathBuf)>; local_raw; children!(
    [path(p)] => (FilePrefix::Home, p)
));

rule_in_group!(absolute_path<(FilePrefix, PathBuf)>; local_raw; children!(
    [path(p)] => (FilePrefix::Absolute, p)
));

// TODO: other import types
rule!(import_type_raw<ImportLocation>; children!(
    // [missing_raw(_e)] => {
    //     ImportLocation::Missing
    // }
    // [env_raw(e)] => {
    //     ImportLocation::Env(e)
    // }
    // [http(url)] => {
    //     ImportLocation::Remote(url)
    // }
    [local_raw((prefix, path))] => {
        ImportLocation::Local(prefix, path)
    }
));

rule!(import_hashed_raw<(ImportLocation, Option<()>)>; children!(
    // TODO: handle hash
    [import_type_raw(import)] => (import, None)
));

rule_group!(expression<RcExpr>);

rule_in_group!(import_raw<RcExpr>; expression; children!(
    // TODO: handle "as Text"
    [import_hashed_raw((location, hash))] => {
        bx(Expr::Embed(Import {
            mode: ImportMode::Code,
            hash,
            location,
        }))
    }
));

rule_in_group!(lambda_expression<RcExpr>; expression; children!(
    [label_raw(l), expression(typ), expression(body)] => {
        bx(Expr::Lam(l, typ, body))
    }
));

rule_in_group!(ifthenelse_expression<RcExpr>; expression; children!(
    [expression(cond), expression(left), expression(right)] => {
        bx(Expr::BoolIf(cond, left, right))
    }
));

rule_in_group!(let_expression<RcExpr>; expression; children!(
    [let_binding(bindings..), expression(final_expr)] => {
        bindings.fold(final_expr, |acc, x| bx(Expr::Let(x.0, x.1, x.2, acc)))
    }
));

rule!(let_binding<(Label, Option<RcExpr>, RcExpr)>; children!(
    [label_raw(name), expression(annot), expression(expr)] => (name, Some(annot), expr),
    [label_raw(name), expression(expr)] => (name, None, expr),
));

rule_in_group!(forall_expression<RcExpr>; expression; children!(
    [label_raw(l), expression(typ), expression(body)] => {
        bx(Expr::Pi(l, typ, body))
    }
));

rule_in_group!(arrow_expression<RcExpr>; expression; children!(
    [expression(typ), expression(body)] => {
        bx(Expr::Pi("_".into(), typ, body))
    }
));

rule_in_group!(merge_expression<RcExpr>; expression; children!(
    [expression(x), expression(y), expression(z)] => bx(Expr::Merge(x, y, Some(z))),
    [expression(x), expression(y)] => bx(Expr::Merge(x, y, None)),
));

rule!(List<()>; raw_pair!(_) => ());
rule!(Optional<()>; raw_pair!(_) => ());

rule_in_group!(empty_collection<RcExpr>; expression; children!(
    [List(_), expression(y)] => {
        bx(Expr::EmptyListLit(y))
    },
    [Optional(_), expression(y)] => {
        bx(Expr::OptionalLit(Some(y), None))
    },
));

rule_in_group!(non_empty_optional<RcExpr>; expression; children!(
    [expression(x), Optional(_), expression(z)] => {
        bx(Expr::OptionalLit(Some(z), Some(x)))
    }
));

rule_in_group!(import_alt_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::ImportAlt, acc, e)))
    },
));
rule_in_group!(or_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::BoolOr, acc, e)))
    },
));
rule_in_group!(plus_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::NaturalPlus, acc, e)))
    },
));
rule_in_group!(text_append_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::TextAppend, acc, e)))
    },
));
rule_in_group!(list_append_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::ListAppend, acc, e)))
    },
));
rule_in_group!(and_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::BoolAnd, acc, e)))
    },
));
rule_in_group!(combine_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::Combine, acc, e)))
    },
));
rule_in_group!(prefer_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::Prefer, acc, e)))
    },
));
rule_in_group!(combine_types_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::CombineTypes, acc, e)))
    },
));
rule_in_group!(times_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::NaturalTimes, acc, e)))
    },
));
rule_in_group!(equal_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::BoolEQ, acc, e)))
    },
));
rule_in_group!(not_equal_expression<RcExpr>; expression; children!(
    [expression(e)] => e,
    [expression(first), expression(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::BinOp(BinOp::BoolNE, acc, e)))
    },
));

rule_in_group!(annotated_expression<RcExpr>; expression; children!(
    [expression(e), expression(annot)] => {
        bx(Expr::Annot(e, annot))
    },
    [expression(e)] => e,
));

rule_in_group!(application_expression<RcExpr>; expression; children!(
    [expression(first), expression(rest..)] => {
        let rest: Vec<_> = rest.collect();
        if rest.is_empty() {
            first
        } else {
            bx(Expr::App(first, rest))
        }
    }
));

rule_in_group!(selector_expression_raw<RcExpr>; expression; children!(
    [expression(first), selector_raw(rest..)] => {
        rest.fold(first, |acc, e| bx(Expr::Field(acc, e)))
    }
));

// TODO: handle record projection
rule!(selector_raw<Label>; children!(
    [label_raw(l)] => l
));

rule_in_group!(literal_expression_raw<RcExpr>; expression; children!(
    [double_literal_raw(n)] => bx(Expr::DoubleLit(n)),
    [minus_infinity_literal(n)] => bx(Expr::DoubleLit(std::f64::NEG_INFINITY)),
    [plus_infinity_literal(n)] => bx(Expr::DoubleLit(std::f64::INFINITY)),
    [NaN_raw(n)] => bx(Expr::DoubleLit(std::f64::NAN)),
    [natural_literal_raw(n)] => bx(Expr::NaturalLit(n)),
    [integer_literal_raw(n)] => bx(Expr::IntegerLit(n)),
    [double_quote_literal(s)] => bx(Expr::TextLit(s)),
    [single_quote_literal(s)] => bx(Expr::TextLit(s)),
    [expression(e)] => e,
));

rule_in_group!(identifier_raw<RcExpr>; expression; children!(
    [label_raw(l), natural_literal_raw(idx)] => {
        let name = String::from(l.clone());
        match Builtin::parse(name.as_str()) {
            Some(b) => bx(Expr::Builtin(b)),
            None => match name.as_str() {
                "True" => bx(Expr::BoolLit(true)),
                "False" => bx(Expr::BoolLit(false)),
                "Type" => bx(Expr::Const(Const::Type)),
                "Kind" => bx(Expr::Const(Const::Kind)),
                _ => bx(Expr::Var(V(l, idx))),
            }
        }
    },
    [label_raw(l)] => {
        let name = String::from(l.clone());
        match Builtin::parse(name.as_str()) {
            Some(b) => bx(Expr::Builtin(b)),
            None => match name.as_str() {
                "True" => bx(Expr::BoolLit(true)),
                "False" => bx(Expr::BoolLit(false)),
                "Type" => bx(Expr::Const(Const::Type)),
                "Kind" => bx(Expr::Const(Const::Kind)),
                _ => bx(Expr::Var(V(l, 0))),
            }
        }
    },
));

rule_in_group!(empty_record_literal<RcExpr>; expression;
    raw_pair!(_) => bx(Expr::RecordLit(BTreeMap::new()))
);

rule_in_group!(empty_record_type<RcExpr>; expression;
    raw_pair!(_) => bx(Expr::Record(BTreeMap::new()))
);

rule_in_group!(non_empty_record_type_or_literal<RcExpr>; expression; children!(
    [label_raw(first_label), non_empty_record_type(rest)] => {
        let (first_expr, mut map) = rest;
        map.insert(first_label, first_expr);
        bx(Expr::Record(map))
    },
    [label_raw(first_label), non_empty_record_literal(rest)] => {
        let (first_expr, mut map) = rest;
        map.insert(first_label, first_expr);
        bx(Expr::RecordLit(map))
    },
));

rule!(non_empty_record_type<(RcExpr, BTreeMap<Label, RcExpr>)>; children!(
    [expression(expr), record_type_entry(entries..)] => {
        (expr, entries.collect())
    }
));

rule!(record_type_entry<(Label, RcExpr)>; children!(
    [label_raw(name), expression(expr)] => (name, expr)
));

rule!(non_empty_record_literal<(RcExpr, BTreeMap<Label, RcExpr>)>; children!(
    [expression(expr), record_literal_entry(entries..)] => {
        (expr, entries.collect())
    }
));

rule!(record_literal_entry<(Label, RcExpr)>; children!(
    [label_raw(name), expression(expr)] => (name, expr)
));

rule_in_group!(union_type_or_literal<RcExpr>; expression; children!(
    [empty_union_type(_)] => {
        bx(Expr::Union(BTreeMap::new()))
    },
    [non_empty_union_type_or_literal((Some((l, e)), entries))] => {
        bx(Expr::UnionLit(l, e, entries))
    },
    [non_empty_union_type_or_literal((None, entries))] => {
        bx(Expr::Union(entries))
    },
));

rule!(empty_union_type<()>; raw_pair!(_) => ());

rule!(non_empty_union_type_or_literal
      <(Option<(Label, RcExpr)>, BTreeMap<Label, RcExpr>)>; children!(
    [label_raw(l), expression(e), union_type_entries(entries)] => {
        (Some((l, e)), entries)
    },
    [label_raw(l), expression(e), non_empty_union_type_or_literal(rest)] => {
        let (x, mut entries) = rest;
        entries.insert(l, e);
        (x, entries)
    },
    [label_raw(l), expression(e)] => {
        let mut entries = BTreeMap::new();
        entries.insert(l, e);
        (None, entries)
    },
));

rule!(union_type_entries<BTreeMap<Label, RcExpr>>; children!(
    [union_type_entry(entries..)] => entries.collect()
));

rule!(union_type_entry<(Label, RcExpr)>; children!(
    [label_raw(name), expression(expr)] => (name, expr)
));

rule_in_group!(non_empty_list_literal_raw<RcExpr>; expression; children!(
    [expression(items..)] => bx(Expr::NEListLit(items.collect()))
));

rule_in_group!(final_expression<RcExpr>; expression; children!(
    [expression(e), EOI(_eoi)] => e
));
}

pub fn parse_expr(s: &str) -> ParseResult<RcExpr> {
    let pairs = DhallParser::parse(Rule::final_expression, s)?;
    // Match the only item in the pairs iterator
    // println!("{}", debug_pair(pairs.clone().next().unwrap()));
    let expr = iter_patterns::destructure_iter!(pairs; [p] => parse_any(p))
        .unwrap()?;
    // expr.expression()
    Ok(expr.expression())
    // Ok(expr)
    // Ok(bx(Expr::BoolLit(false)))
}

#[test]
fn test_parse() {
    // let expr = r#"{ x = "foo", y = 4 }.x"#;
    // let expr = r#"(1 + 2) * 3"#;
    let expr = r#"(1) + 3 * 5"#;
    println!("{:?}", parse_expr(expr));
    match parse_expr(expr) {
        Err(e) => {
            println!("{:?}", e);
            println!("{}", e);
        }
        ok => println!("{:?}", ok),
    };
    // assert!(false);
}
