//! Zeron has no math typesetter, so TeX renders as its closest Unicode text.

pub fn tex_to_text(tex: &str) -> String {
    let chars: Vec<char> = tex.chars().collect();
    let mut i = 0;
    let out = convert(&chars, &mut i, None);
    collapse_spaces(&out)
}

fn convert(cs: &[char], i: &mut usize, stop: Option<char>) -> String {
    let mut out = String::new();
    while *i < cs.len() {
        let c = cs[*i];
        if Some(c) == stop {
            *i += 1;
            break;
        }
        *i += 1;
        match c {
            '{' => out.push_str(&convert(cs, i, Some('}'))),
            '\\' => command(cs, i, &mut out),
            '^' | '_' => {
                let arg = argument(cs, i);
                out.push_str(&script(&arg, c == '^'));
            }
            '~' => out.push(' '),
            '\n' | '\r' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

fn argument(cs: &[char], i: &mut usize) -> String {
    while *i < cs.len() && cs[*i] == ' ' {
        *i += 1;
    }
    let Some(&c) = cs.get(*i) else {
        return String::new();
    };
    *i += 1;
    match c {
        '{' => convert(cs, i, Some('}')),
        '\\' => {
            let mut out = String::new();
            command(cs, i, &mut out);
            out
        }
        _ => c.to_string(),
    }
}

fn command(cs: &[char], i: &mut usize, out: &mut String) {
    let Some(&first) = cs.get(*i) else {
        out.push('\\');
        return;
    };
    if !first.is_ascii_alphabetic() {
        *i += 1;
        match first {
            ',' | ':' | ';' | ' ' => out.push(' '),
            '!' => {}
            '\\' => out.push(' '),
            _ => out.push(first),
        }
        return;
    }
    let start = *i;
    while *i < cs.len() && cs[*i].is_ascii_alphabetic() {
        *i += 1;
    }
    let name: String = cs[start..*i].iter().collect();
    match name.as_str() {
        "text" | "textrm" | "textbf" | "textit" | "textsf" | "texttt" | "mathrm" | "mathbf"
        | "mathit" | "mathsf" | "mathtt" | "mathcal" | "boldsymbol" | "bm" | "mbox"
        | "operatorname" => out.push_str(&argument(cs, i)),
        "mathbb" => {
            let arg = argument(cs, i);
            out.extend(arg.chars().map(double_struck));
        }
        "frac" | "dfrac" | "tfrac" => {
            let num = argument(cs, i);
            let den = argument(cs, i);
            out.push_str(&format!("{}/{}", wrap(&num), wrap(&den)));
        }
        "sqrt" => {
            let arg = argument(cs, i);
            out.push('√');
            out.push_str(&wrap(&arg));
        }
        "hat" | "widehat" => combine(out, &argument(cs, i), '\u{0302}'),
        "bar" | "overline" => combine(out, &argument(cs, i), '\u{0304}'),
        "tilde" | "widetilde" => combine(out, &argument(cs, i), '\u{0303}'),
        "dot" => combine(out, &argument(cs, i), '\u{0307}'),
        "vec" => combine(out, &argument(cs, i), '\u{20D7}'),
        "left" | "right" | "bigl" | "bigr" | "Bigl" | "Bigr" | "big" | "Big" | "bigg" | "Bigg" => {
            if cs.get(*i) == Some(&'.') {
                *i += 1;
            }
        }
        "pmod" => {
            let arg = argument(cs, i);
            out.push_str(&format!(" (mod {arg})"));
        }
        "displaystyle" | "textstyle" | "limits" | "nolimits" => {}
        _ => match symbol(&name) {
            Some(s) => out.push_str(s),
            None => {
                out.push('\\');
                out.push_str(&name);
            }
        },
    }
}

fn wrap(s: &str) -> String {
    let s = s.trim();
    if s.chars().all(|c| c.is_alphanumeric() || c == '.') {
        s.to_string()
    } else {
        format!("({s})")
    }
}

fn combine(out: &mut String, arg: &str, mark: char) {
    for c in arg.chars() {
        out.push(c);
        if !c.is_whitespace() {
            out.push(mark);
        }
    }
}

fn script(arg: &str, sup: bool) -> String {
    let arg = arg.trim();
    let mapped: Option<String> = arg
        .chars()
        .map(|c| if sup { superscript(c) } else { subscript(c) })
        .collect();
    match mapped {
        Some(s) if !s.is_empty() => s,
        _ => {
            let mark = if sup { '^' } else { '_' };
            if arg.chars().count() == 1 {
                format!("{mark}{arg}")
            } else {
                format!("{mark}({arg})")
            }
        }
    }
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(c);
    }
    out.truncate(out.trim_end().len());
    out
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'i' => 'ⁱ',
        'n' => 'ⁿ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'k' => 'ᵏ',
        'T' => 'ᵀ',
        '′' => '′',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'x' => 'ₓ',
        _ => return None,
    })
}

fn double_struck(c: char) -> char {
    match c {
        'N' => 'ℕ',
        'Z' => 'ℤ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'C' => 'ℂ',
        _ => c,
    }
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "implies" | "Longrightarrow" => "⟹",
        "impliedby" | "Longleftarrow" => "⟸",
        "iff" | "Longleftrightarrow" => "⟺",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔",
        "to" | "rightarrow" => "→",
        "gets" | "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "neq" | "ne" => "≠",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "ll" => "≪",
        "gg" => "≫",
        "approx" => "≈",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "equiv" => "≡",
        "propto" => "∝",
        "times" => "×",
        "cdot" => "⋅",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "•",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "lnot" | "neg" => "¬",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "emptyset" | "varnothing" => "∅",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "oint" => "∮",
        "ldots" | "dots" | "dotsc" | "dotsb" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "angle" => "∠",
        "perp" | "bot" => "⊥",
        "top" => "⊤",
        "parallel" => "∥",
        "mid" => "∣",
        "therefore" => "∴",
        "because" => "∵",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "vert" | "lvert" | "rvert" => "|",
        "Vert" | "lVert" | "rVert" => "‖",
        "lbrace" => "{",
        "rbrace" => "}",
        "backslash" => "\\",
        "prime" => "′",
        "degree" => "°",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "aleph" => "ℵ",
        "quad" => "  ",
        "qquad" => "    ",
        "bmod" | "mod" => " mod ",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "log" => "log",
        "ln" => "ln",
        "exp" => "exp",
        "lim" => "lim",
        "max" => "max",
        "min" => "min",
        "det" => "det",
        "gcd" => "gcd",
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::tex_to_text;

    #[test]
    fn logic_puzzle_thread_reads_as_text() {
        assert_eq!(tex_to_text(r"\text{T}"), "T");
        assert_eq!(
            tex_to_text(r"\text{D} \implies A \neq C \implies S_B"),
            "D ⟹ A ≠ C ⟹ S_B"
        );
        assert_eq!(tex_to_text(r"2^3 = 8"), "2³ = 8");
        assert_eq!(tex_to_text(r"P \in \{-1, +1\}"), "P ∈ {-1, +1}");
        assert_eq!(tex_to_text(r"X \cdot Y = +1"), "X ⋅ Y = +1");
        assert_eq!(
            tex_to_text(r"\mathbf{A \cdot B \cdot C = -1}"),
            "A ⋅ B ⋅ C = -1"
        );
        assert_eq!(
            tex_to_text(r"-1 = A \cdot B \cdot C = +1 \quad (\text{Vô lý})"),
            "-1 = A ⋅ B ⋅ C = +1 (Vô lý)"
        );
        assert_eq!(tex_to_text(r"\text{Nói thật} = +1"), "Nói thật = +1");
    }

    #[test]
    fn scripts_fall_back_when_unicode_has_no_glyph() {
        assert_eq!(tex_to_text("S_A"), "S_A");
        assert_eq!(tex_to_text("x_{AB}"), "x_(AB)");
        assert_eq!(tex_to_text("a_1 + x^{n+1}"), "a₁ + xⁿ⁺¹");
        assert_eq!(tex_to_text("e^{i\\pi}"), "e^(iπ)");
    }

    #[test]
    fn structures_and_unknown_commands() {
        assert_eq!(tex_to_text(r"\frac{1}{2}"), "1/2");
        assert_eq!(tex_to_text(r"\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(tex_to_text(r"\sqrt{x+1}"), "√(x+1)");
        assert_eq!(tex_to_text(r"\left( x \right)"), "( x )");
        assert_eq!(tex_to_text(r"\mathbb{R}"), "ℝ");
        assert_eq!(tex_to_text(r"\hat{x}"), "x\u{0302}");
        assert_eq!(tex_to_text(r"\foo + 1"), r"\foo + 1");
        assert_eq!(tex_to_text(r"\alpha\beta"), "αβ");
    }
}
