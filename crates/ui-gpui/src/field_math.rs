//! 数字框里输入的文字是什么意思：一个数字、一段算术（`+ - * /`、括号，数字后可带 `px`、`°`、`%`
//! 之类的单位），或相对当前值的编辑（`+10`、`*2`、`/2`，以及非百分比字段里的 `50%`）。
//!
//! 移植自上游 `ui-egui/src/widgets/field_math.rs`（含测试表）。GPUI 这边多一个显式参数
//! `percent_unit`：上游由调用方用 `parse(text, suffix == "%")` 传入，这里同样由字段的后缀决定。

/// 相对字段当前值的编辑：`+10`、`*2`、`/4`、`50%` 各自代表什么。字段显示 Mixed（没有单一值）
/// 时，上游把它落到每个图层自己的值上；GPUI 的字段是逐图层的，调用方自行处理。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rel {
    Add(f64),
    Mul(f64),
    Div(f64),
}

impl Rel {
    /// `cur` 变成的值（除零等无意义结果返回 `None`）。
    pub fn apply(self, cur: f64) -> Option<f64> {
        let v = match self {
            Rel::Add(b) => cur + b,
            Rel::Mul(b) => cur * b,
            Rel::Div(b) if b != 0.0 => cur / b,
            Rel::Div(_) => return None,
        };
        v.is_finite().then_some(v)
    }
}

/// 字段里的文字含义。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Typed {
    Value(f64),
    Rel(Rel),
}

/// 超过这个长度的不当作有人输入的数字。
const MAX_LEN: usize = 256;
/// 括号最多嵌套这么深。
const MAX_DEPTH: usize = 32;

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// 数字后的 `%` 是它的单位（百分比字段）；否则不参与算术。
    percent_unit: bool,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }

    fn eat(&mut self, c: char) -> bool {
        self.space();
        self.chars.next_if_eq(&c).is_some()
    }

    /// 乘积的加减。
    fn sum(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.product(depth)?;
        loop {
            if self.eat('+') {
                v += self.product(depth)?;
            } else if self.eat('-') {
                v -= self.product(depth)?;
            } else {
                return v.is_finite().then_some(v);
            }
        }
    }

    fn product(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.factor(depth)?;
        loop {
            if self.eat('*') {
                v *= self.factor(depth)?;
            } else if self.eat('/') {
                let d = self.factor(depth)?;
                if d == 0.0 {
                    return None;
                }
                v /= d;
            } else {
                return Some(v);
            }
        }
    }

    /// 带单位的数字、括号里的和，或它们的负数。
    fn factor(&mut self, depth: usize) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        if self.eat('-') {
            return self.factor(depth + 1).map(|v| -v);
        }
        if self.eat('(') {
            let v = self.sum(depth + 1)?;
            return self.eat(')').then_some(v);
        }
        let mut digits = String::new();
        while let Some(c) = self.chars.next_if(|c| c.is_ascii_digit() || matches!(c, '.' | ',')) {
            // 十进制逗号当小数点。
            digits.push(if c == ',' { '.' } else { c });
        }
        let v = digits.parse::<f64>().ok().filter(|v| v.is_finite())?;
        // 单位：字母（px、pt、ms、x）、度数符号，以及百分比字段里的百分号。可以跟在空格后（"3 px"）。
        let percent_unit = self.percent_unit;
        let is_unit = |c: &char| c.is_ascii_alphabetic() || *c == '°' || (*c == '%' && percent_unit);
        let mut ahead = self.chars.clone();
        while ahead.next_if(|c| c.is_whitespace()).is_some() {}
        if ahead.peek().is_some_and(is_unit) {
            self.space();
        }
        while self.chars.next_if(is_unit).is_some() {}
        Some(v)
    }

    fn end(&mut self) -> bool {
        self.space();
        self.chars.peek().is_none()
    }
}

/// `text` 输入数字框后是什么意思。`percent_unit`：字段显示百分比，数字后的 `%` 只是单位；
/// 否则末尾的 `%` 让这次编辑变成相对值（`50%` = 一半，`+10%` = 多一成，`-10%` = 少一成）。
pub fn parse(text: &str, percent_unit: bool) -> Option<Typed> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_LEN {
        return None;
    }
    let percent = (!percent_unit).then(|| text.strip_suffix('%')).flatten();
    let body = percent.unwrap_or(text).trim();
    // 开头是什么决定它是不是相对编辑（"-4" 仍是数值减四）。
    let (lead, rest) = match body.chars().next() {
        Some(c @ ('+' | '*' | '/')) => (Some(c), body.get(c.len_utf8()..).unwrap_or("")),
        _ => (None, body),
    };
    let mut p = Parser { chars: rest.chars().peekable(), percent_unit };
    let v = p.sum(0).filter(|_| p.end())?;
    let rel = match (percent.is_some(), lead) {
        (false, None) => return Some(Typed::Value(v)),
        (false, Some('+')) => Rel::Add(v),
        (false, Some('*')) => Rel::Mul(v),
        (false, Some(_)) => Rel::Div(v),
        // "50%" = 值的一半，"+10%" = 多一成（打出的减号在 `v` 里："-10%" 是少一成）。
        (true, None) if body.starts_with('-') => Rel::Mul(1.0 + v / 100.0),
        (true, None) => Rel::Mul(v / 100.0),
        (true, Some('+')) => Rel::Mul(1.0 + v / 100.0),
        (true, Some('*')) => Rel::Mul(v / 100.0),
        (true, Some(_)) => Rel::Div(v / 100.0),
    };
    Some(Typed::Rel(rel))
}

/// `text` 给值为 `cur` 的字段的数字（`cur` 为 `None`：没有单一值）。没有值时 `+4` 是 4，
/// 其它相对编辑无意义。
pub fn eval(text: &str, cur: Option<f64>, percent_unit: bool) -> Option<f64> {
    match (parse(text, percent_unit)?, cur) {
        (Typed::Value(v), _) => Some(v),
        (Typed::Rel(r), Some(c)) => r.apply(c),
        (Typed::Rel(Rel::Add(v)), None) => Some(v),
        (Typed::Rel(_), None) => None,
    }
}

/// 从 `from` 走 `steps` 步、每步 `step`，去掉反复相加的浮点尘（0.1 + 0.2）。
pub fn stepped(from: f64, steps: f64, step: f64) -> f64 {
    let v = from + steps * step;
    if !v.is_finite() {
        return from;
    }
    let rounded = (v * 1e6).round() / 1e6;
    // 超过约 1.8e302 时缩放会溢出；那样的值本来也没有尘可去。
    if rounded.is_finite() { rounded } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_after_a_space() {
        assert_eq!(parse("3px", true), Some(Typed::Value(3.0)));
        assert_eq!(parse("3 px", true), Some(Typed::Value(3.0)));
        assert_eq!(parse("10 °", true), Some(Typed::Value(10.0)));
        assert_eq!(parse("50 %", true), Some(Typed::Value(50.0)));
        assert_eq!(parse("3 px + 2 px", true), Some(Typed::Value(5.0)));
    }

    #[test]
    fn stepped_keeps_a_huge_finite_value_finite() {
        assert_eq!(stepped(1e305, 1.0, 1.0), 1e305);
        assert!(stepped(f64::MAX / 2.0, 0.0, 1.0).is_finite());
        assert_eq!(stepped(0.1, 1.0, 0.2), 0.3);
        assert_eq!(stepped(1.0, 1.0, f64::INFINITY), 1.0);
    }

    /// 与上游 `field_tests::typed_text_is_a_number_arithmetic_or_an_edit_relative_to_the_value` 同一张表。
    #[test]
    fn typed_text_is_a_number_arithmetic_or_an_edit_relative_to_the_value() {
        use Typed::{Rel as R, Value as V};
        let rows: Vec<(&str, bool, Option<Typed>)> = vec![
            ("12", false, Some(V(12.0))),
            (" 12.5 ", false, Some(V(12.5))),
            ("12,5", false, Some(V(12.5))),
            (".5", false, Some(V(0.5))),
            ("-3", false, Some(V(-3.0))),
            ("- 3", false, Some(V(-3.0))),
            ("10+5", false, Some(V(15.0))),
            ("10 - 3", false, Some(V(7.0))),
            ("2*3+4", false, Some(V(10.0))),
            ("2+3*4", false, Some(V(14.0))),
            ("(2+3)*4", false, Some(V(20.0))),
            ("100/(2+3)", false, Some(V(20.0))),
            ("-(4+1)", false, Some(V(-5.0))),
            ("((1+2)*(3+4))/7", false, Some(V(3.0))),
            ("2*-3", false, Some(V(-6.0))),
            ("10--2", false, Some(V(12.0))),
            ("12px", false, Some(V(12.0))),
            ("45°", false, Some(V(45.0))),
            ("2x", false, Some(V(2.0))),
            ("300ms", false, Some(V(300.0))),
            ("10px + 2px", false, Some(V(12.0))),
            ("+10", false, Some(R(Rel::Add(10.0)))),
            ("+ 2*3", false, Some(R(Rel::Add(6.0)))),
            ("*2", false, Some(R(Rel::Mul(2.0)))),
            ("/4", false, Some(R(Rel::Div(4.0)))),
            ("*(1+1)", false, Some(R(Rel::Mul(2.0)))),
            ("50%", false, Some(R(Rel::Mul(0.5)))),
            ("+10%", false, Some(R(Rel::Mul(1.1)))),
            ("-10%", false, Some(R(Rel::Mul(0.9)))),
            ("50%", true, Some(V(50.0))),
            ("+10%", true, Some(R(Rel::Add(10.0)))),
            ("50% + 10%", true, Some(V(60.0))),
            ("", false, None),
            ("   ", false, None),
            ("abc", false, None),
            ("1/0", false, None),
            ("/0", false, Some(R(Rel::Div(0.0)))),
            ("(1+2", false, None),
            ("1+2)", false, None),
            ("1+", false, None),
            ("*", false, None),
            ("1 2", false, None),
            ("1..2", false, None),
            ("50%%", false, None),
            ("5%+1", false, None),
            ("1e999", false, None),
            ("٣", false, None),
        ];
        for (text, pct, want) in rows {
            let got = parse(text, pct);
            match (got, want) {
                (Some(V(a)), Some(V(b)))
                | (Some(R(Rel::Add(a))), Some(R(Rel::Add(b))))
                | (Some(R(Rel::Mul(a))), Some(R(Rel::Mul(b))))
                | (Some(R(Rel::Div(a))), Some(R(Rel::Div(b)))) => {
                    assert!((a - b).abs() < 1e-9, "{text:?}: {got:?} (wanted {want:?})");
                }
                (None, None) => {}
                _ => panic!("{text:?} (percentage field: {pct}): {got:?} (wanted {want:?})"),
            }
        }
        // 相对编辑得到的值，以及没有值时什么也不给。
        assert_eq!(Rel::Add(10.0).apply(5.0), Some(15.0));
        assert_eq!(Rel::Mul(2.0).apply(5.0), Some(10.0));
        assert_eq!(Rel::Div(4.0).apply(10.0), Some(2.5));
        assert_eq!(Rel::Div(0.0).apply(10.0), None);
        assert_eq!(Rel::Mul(f64::MAX).apply(f64::MAX), None);
        // `eval`：单位按单位读；没有值时 `+4` 是 4。
        for (text, cur, want) in [
            ("12", None, Some(12.0)),
            ("10+5", None, Some(15.0)),
            ("*2", Some(8.0), Some(16.0)),
            ("+4", Some(8.0), Some(12.0)),
            ("+4", None, Some(4.0)),
            ("*2", None, None),
            ("-3", None, Some(-3.0)),
            ("50%", None, Some(50.0)),
            ("1/0", None, None),
            ("abc", None, None),
        ] {
            assert_eq!(eval(text, cur, true), want, "eval({text:?}, {cur:?})");
        }
        // 乱七八糟的输入不 panic：超长、过深、怪字符。
        for junk in ["(".repeat(5000), "1+".repeat(5000), format!("{}1{}", "(".repeat(40), ")".repeat(40)), "−1".into(), "1\u{0}2".into(), "é%".into()]
        {
            let _ = parse(&junk, false);
            let _ = parse(&junk, true);
        }
    }
}
