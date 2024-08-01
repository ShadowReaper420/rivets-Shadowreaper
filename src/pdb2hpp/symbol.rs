use std::{
    cell::{Cell, RefCell},
    fmt::Display,
};

use lazy_regex::{regex, regex_is_match};

/// The full type name including namespaces and template types. ie `std::vector<int>`
#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Symbol(String),
    String(String),
    None,
}

/// A repersentation of a c++ typename. For example: `std::vector<int>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    name: Type,
    pointer_count: Cell<usize>,
    modifiers: RefCell<String>,
}

impl Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

const NONETYPE_ERROR: &str = "/* error: attempt to display invalid type */";

// This impl block contains all the `static` functions on the Symbol class.
impl Symbol {
    const fn _new(name: Type) -> Self {
        Self {
            name,
            pointer_count: Cell::new(0),
            modifiers: RefCell::new(String::new()),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn new(name: String) -> Self {
        let unlambdad = Self::replace_unnamed_types(&name);
        Self::_new(Type::Symbol(unlambdad))
    }

    pub const fn from_string(name: String) -> Self {
        Self::_new(Type::String(name))
    }

    pub const fn none() -> Self {
        Self::_new(Type::None)
    }

    pub fn replace_unnamed_types(name: &str) -> String {
        let name = name.replace("<unnamed-tag>", "unnamed_tag");
        let name = regex!(r"<lambda_(\w+?)>").replace_all(&name, "lambda_$1")
        .into_owned();

        regex!(r"<unnamed-(type|enum)-(.+?)>")
            .replace_all(&name, |captures: &regex::Captures| {
                captures
                    .get(2)
                    .expect("Compiled regex will always have a capture group")
                    .as_str()
                    .to_owned()
            })
            .into_owned()
    }
}

// This impl block contains all the instance functions on the Symbol class.
impl Symbol {
    pub fn as_str(&self) -> &str {
        match &self.name {
            Type::Symbol(s) => {
                let without_templates = s.find('<').map_or_else(|| s.as_str(), |i| &s[..i]);
                assert!(!without_templates.ends_with("::"), "Symbol::as_str() called on a symbol with trailing '::': {s}");
                without_templates
            },
            Type::String(s) => s,
            Type::None => NONETYPE_ERROR,
        }
    }

    pub fn increment_pointer_count(&self) {
        self.pointer_count.set(self.pointer_count.get() + 1);
    }

    pub fn add_modifiers(&self, modifiers: &str) {
        self.modifiers.borrow_mut().push_str(modifiers);
    }

    pub fn fully_qualifed(&self) -> String {
        match &self.name {
            Type::Symbol(s) => {
                let pointers = self.pointer_count.get();
                let pointers = "*".repeat(pointers);
                let modifiers = &self.modifiers.borrow();

                format!("{modifiers}{s}{pointers}")
            }
            Type::String(s) => s.to_string(),
            Type::None => NONETYPE_ERROR.to_string(),
        }
    }

    pub fn namespace_vec(&self) -> Vec<String> {
        let s = self.as_str();

        let mut result = Vec::new();
        let mut current = String::new();
        let mut template_depth = 0;

        let chars: Vec<char> = s.chars().collect();
        let mut skip = false;

        for (i, char) in chars.iter().enumerate() {
            if skip {
                skip = false;
                continue;
            }
            match char {
                '<' => {
                    template_depth += 1;
                    current.push('<');
                }
                '>' => {
                    template_depth -= 1;
                    current.push('>');
                }
                ':' if template_depth == 0 && i + 1 < chars.len() && chars[i + 1] == ':' => {
                    result.push(current.trim().to_string());
                    current.clear();
                    skip = true; // skip the next ':'
                }
                _ => {
                    current.push(chars[i]);
                }
            }
        }

        if !current.is_empty() {
            result.push(current.trim().to_string());
        }

        result
    }

    /// We need to split template types by commas, however sometimes nested templates contain commas inside <>.
    /// This is a special implementation of the `split()` function to handle the above case.
    fn templates_vec(&self) -> Vec<String> {
        let Type::Symbol(type_name) = &self.name else {
            return Vec::new();
        };

        let re = regex!(r"(.+?)<(.*)>");
        let Some(captures) = re.captures(type_name) else {
            return Vec::new();
        };

        let class_name_without_templates = captures
            .get(1)
            .expect("Static regex always has one group")
            .as_str();

        // This handles the special case of my_namespace::<unnamed-class-MyClass>
        if class_name_without_templates.ends_with("::") {
            return Vec::new();
        }

        let templates = captures
            .get(2)
            .expect("Static regex always has two groups")
            .as_str();

        let mut result = Vec::new();
        let mut current = String::new();
        let mut template_depth = 0;

        let chars: Vec<char> = templates.chars().collect();

        for char in chars {
            match char {
                '<' | '(' => {
                    template_depth += 1;
                    current.push(char);
                }
                '>' | ')' => {
                    template_depth -= 1;
                    if template_depth < 0 {
                        template_depth = 0;
                    }
                    current.push(char);
                }
                ',' if template_depth == 0 => {
                    result.push(current);
                    current = String::with_capacity(32);
                }
                _ => {
                    current.push(char);
                }
            }
        }

        if !current.is_empty() {
            result.push(current);
        }

        result
    }

    /// Figures out the template types based on the class name stored in the PDB.
    /// For example `my_namespace::MyClass<int, std::string>` would return:
    /// `[("typename", "T", "int"), ("typename", "U", std::string)]`
    pub fn templates_by_type(&self) -> Vec<(String, String, String)> {
        /// Converts a number into base 7 where each digit is a letter from T, U, V, W, X, Y, Z.
        fn number_to_template_type_name(i: usize) -> String {
            match i {
                0 => "T".to_string(),
                1 => "U".to_string(),
                2 => "V".to_string(),
                3 => "W".to_string(),
                4 => "X".to_string(),
                5 => "Y".to_string(),
                6 => "Z".to_string(),
                _ => {
                    let remainder = i % 7;
                    let i = i / 7;
                    let mut s = number_to_template_type_name(i);
                    s.push_str(&number_to_template_type_name(remainder));
                    s
                }
            }
        }

        self.templates_vec()
            .into_iter()
            .enumerate()
            .map(|(i, template)| {
                let template_keyword = if regex_is_match!(r"^\d+\.\d+$", &template) {
                    "double"
                } else if regex_is_match!(r"^\d+$", &template) {
                    "long long"
                } else {
                    "typename"
                }
                .to_string();

                let identifier = number_to_template_type_name(i);

                (template_keyword, identifier, template)
            })
            .collect()
    }

    /// Returns a string that would be used on this class to define its template classes.
    /// For example `template <typename T, typename U>`, `template<typename T, int L>`, `template<typename T, template<typename U, int I> class Arr>`, or `template <typename... Arguments>`.
    /// Disclaimer: I do not support this nonsense: `template<class T, T::type n = 0>`.
    pub fn templates(templates_by_type: &[(String, String, String)]) -> String {
        let mut result = String::new();

        if templates_by_type.is_empty() {
            return result;
        }

        result.push_str("template <");
        for (i, (template_keyword, identifier, _)) in templates_by_type.iter().enumerate() {
            if i != 0 {
                result.push_str(", ");
            }

            result.push_str(template_keyword);
            result.push(' ');
            result.push_str(identifier);
        }

        result.push_str("> ");
        result
    }
}
