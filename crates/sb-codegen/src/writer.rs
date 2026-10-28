//! Tiny indenting writer used to emit readable code.

#[derive(Default)]
pub struct Code {
    buf: String,
    level: usize,
}

impl Code {
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes one or several lines at the current indentation. Empty lines stay empty.
    pub fn line(&mut self, text: impl AsRef<str>) -> &mut Self {
        for l in text.as_ref().split('\n') {
            if !l.trim().is_empty() {
                for _ in 0..self.level {
                    self.buf.push_str("    ");
                }
                self.buf.push_str(l);
            }
            self.buf.push('\n');
        }
        self
    }

    pub fn blank(&mut self) -> &mut Self {
        self.buf.push('\n');
        self
    }

    /// Bare `{` block, then indents.
    pub fn block(&mut self) -> &mut Self {
        self.line("{");
        self.level += 1;
        self
    }

    pub fn indent(&mut self) -> &mut Self {
        self.level += 1;
        self
    }

    pub fn dedent(&mut self) -> &mut Self {
        self.level = self.level.saturating_sub(1);
        self
    }

    /// `header {` then indents.
    pub fn open(&mut self, header: impl AsRef<str>) -> &mut Self {
        self.line(format!("{} {{", header.as_ref()));
        self.level += 1;
        self
    }

    /// Dedents then `}` followed by `suffix` (for `} else {` use [`Code::close_open`]).
    pub fn close(&mut self) -> &mut Self {
        self.level = self.level.saturating_sub(1);
        self.line("}")
    }

    pub fn close_with(&mut self, suffix: &str) -> &mut Self {
        self.level = self.level.saturating_sub(1);
        self.line(format!("}}{suffix}"))
    }

    /// `} else {` style transition.
    pub fn close_open(&mut self, header: impl AsRef<str>) -> &mut Self {
        self.level = self.level.saturating_sub(1);
        self.line(format!("}} {} {{", header.as_ref()));
        self.level += 1;
        self
    }

    pub fn finish(self) -> String {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_blocks() {
        let mut c = Code::new();
        c.open("fn main()");
        c.line("a;\nb;");
        c.open("if x");
        c.line("c;");
        c.close_open("else");
        c.line("d;");
        c.close();
        c.close();
        assert_eq!(
            c.finish(),
            "fn main() {\n    a;\n    b;\n    if x {\n        c;\n    } else {\n        d;\n    }\n}\n"
        );
    }
}
