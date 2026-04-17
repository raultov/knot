use tree_sitter::{Language, Parser};

fn main() {
    let mut parser = Parser::new();
    let language = tree_sitter_css::LANGUAGE;
    parser.set_language(&language.into()).unwrap();

    let source = "#header-container { color: red; }";
    let tree = parser.parse(source, None).unwrap();

    print_node(tree.root_node(), 0);
}

fn print_node(node: tree_sitter::Node, depth: usize) {
    println!(
        "{}{} [{} - {}]",
        "  ".repeat(depth),
        node.kind(),
        node.start_byte(),
        node.end_byte()
    );
    for i in 0..node.child_count() {
        print_node(node.child(i).unwrap(), depth + 1);
    }
}
