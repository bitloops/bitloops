#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphqlArgument {
    pub(super) name: String,
    pub(super) value: String,
}

impl GraphqlArgument {
    pub(super) fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GraphqlSelection {
    Scalar(String),
    Field(GraphqlField),
}

impl GraphqlSelection {
    pub(super) fn scalar(name: impl Into<String>) -> Self {
        Self::Scalar(name.into())
    }
}

impl From<GraphqlField> for GraphqlSelection {
    fn from(value: GraphqlField) -> Self {
        Self::Field(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphqlField {
    pub(super) name: String,
    pub(super) args: Vec<GraphqlArgument>,
    pub(super) selection_set: Vec<GraphqlSelection>,
}

impl GraphqlField {
    pub(super) fn new(
        name: impl Into<String>,
        args: Vec<GraphqlArgument>,
        selection_set: Vec<GraphqlSelection>,
    ) -> Self {
        Self {
            name: name.into(),
            args,
            selection_set,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphqlDocumentBuilder {
    root_fields: Vec<GraphqlField>,
}

impl GraphqlDocumentBuilder {
    pub(super) fn new(root_fields: Vec<GraphqlField>) -> Self {
        Self { root_fields }
    }

    pub(super) fn build(&self) -> String {
        let mut lines = vec!["query {".to_string()];
        for field in &self.root_fields {
            render_field(field, 1, &mut lines);
        }
        lines.push("}".to_string());
        lines.join("\n")
    }
}

fn render_field(field: &GraphqlField, depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let args = render_args(&field.args);
    if field.selection_set.is_empty() {
        lines.push(format!("{indent}{}{args}", field.name));
        return;
    }

    lines.push(format!("{indent}{}{args} {{", field.name));
    for selection in &field.selection_set {
        match selection {
            GraphqlSelection::Scalar(name) => {
                lines.push(format!("{}{}", "  ".repeat(depth + 1), name));
            }
            GraphqlSelection::Field(child) => render_field(child, depth + 1, lines),
        }
    }
    lines.push(format!("{indent}}}"));
}

fn render_args(args: &[GraphqlArgument]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let rendered = args
        .iter()
        .map(|arg| format!("{}: {}", arg.name, arg.value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("({rendered})")
}
