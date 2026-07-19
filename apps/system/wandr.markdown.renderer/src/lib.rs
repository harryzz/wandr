//! Markdown renderer component — task 36 step 2.
//!
//! Implements `wandr:markdown/renderer@0.1.0`'s `render` export. Parses
//! CommonMark via `pulldown-cmark` and emits the WIT `document` tree
//! the consumer can drop into a Compose `LazyColumn` / `AnnotatedString`.
//!
//! Why pulldown-cmark and not comrak: comrak transitively depends on
//! `slug`, which pulls `wasm-bindgen` (JS-interop), which doesn't
//! compile to `wasm32-wasip2`. pulldown-cmark is pure-Rust with no
//! JS-host assumptions.
//!
//! WIT 0.2 doesn't allow recursive types, so:
//!   - Inline spans flatten into `run { text; styles; link-url }`
//!     records; nested emphasis collapses into a multi-element styles
//!     list on the same run.
//!   - Block nesting is one level deep — top-level `block` hosts lists
//!     + block-quotes whose contents use the `simple-block` subset.
//!     Nested-lists / nested-quotes get flattened.

wit_bindgen::generate!({
    world: "renderer-world",
    // Single .wit file — pointing at the directory would also parse
    // skiko-gfx.wit, which wit-bindgen 0.46 rejects on `matrix-3x3`
    // (numeric chars mid-identifier — wasmtime's host bindgen accepts
    // them, but the guest bindgen is stricter on the current spec).
    path: "../../../contracts/wit/markdown.wit",
});

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use exports::wandr::markdown::renderer::{
    Block, CodeBlock, Document, Guest, HeadingBlock, InlineStyle, ListItem,
    OrderedListBlock, Run, SimpleBlock,
};

struct MarkdownRenderer;

impl Guest for MarkdownRenderer {
    fn render(source: String) -> Document {
        let root = build_tree(&source);
        let mut blocks: Vec<Block> = Vec::new();
        for child in &root.children {
            collect_block(child, &mut blocks);
        }
        Document { blocks }
    }
}

export!(MarkdownRenderer);

// ── tiny AST built from the pulldown-cmark event stream ─────────────

enum NodeKind {
    Document,
    Paragraph,
    Heading(u8),
    CodeBlock(Option<String>, String),   // (language, accumulated text)
    BlockQuote,
    BulletList,
    OrderedList(u32),                    // start number
    Item,
    ThematicBreak,
    Emphasis,
    Strong,
    Link(String),                        // dest URL
    Text(String),
    InlineCode(String),
    SoftBreak,
    HardBreak,
}

struct Node {
    kind:     NodeKind,
    children: Vec<Node>,
}

impl Node {
    fn new(kind: NodeKind) -> Self { Self { kind, children: Vec::new() } }
}

fn build_tree(source: &str) -> Node {
    let mut stack: Vec<Node> = vec![Node::new(NodeKind::Document)];
    let mut in_code_block: bool = false;

    for event in Parser::new(source) {
        match event {
            Event::Start(tag) => {
                if let Some(kind) = block_or_inline_from_tag(&tag) {
                    if matches!(kind, NodeKind::CodeBlock(_, _)) {
                        in_code_block = true;
                    }
                    stack.push(Node::new(kind));
                }
            }
            Event::End(end) => {
                if matches!(end, TagEnd::CodeBlock) {
                    in_code_block = false;
                }
                if stack.len() > 1 {
                    let node = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(node);
                }
            }
            Event::Text(s) => {
                let parent = stack.last_mut().unwrap();
                if in_code_block {
                    // CodeBlock holds its literal text on the node itself,
                    // not as Text children.
                    if let NodeKind::CodeBlock(_, ref mut buf) = parent.kind {
                        buf.push_str(&s);
                    }
                } else {
                    parent.children.push(Node::new(NodeKind::Text(s.into_string())));
                }
            }
            Event::Code(s) => {
                stack.last_mut().unwrap().children
                    .push(Node::new(NodeKind::InlineCode(s.into_string())));
            }
            Event::SoftBreak => {
                stack.last_mut().unwrap().children
                    .push(Node::new(NodeKind::SoftBreak));
            }
            Event::HardBreak => {
                stack.last_mut().unwrap().children
                    .push(Node::new(NodeKind::HardBreak));
            }
            Event::Rule => {
                stack.last_mut().unwrap().children
                    .push(Node::new(NodeKind::ThematicBreak));
            }
            // Tables, footnotes, HTML, task-list markers, math — not in
            // the v0.1 WIT. Skipped silently; their text content still
            // flows through the inline channels above.
            _ => {}
        }
    }

    stack.pop().unwrap()
}

fn block_or_inline_from_tag(tag: &Tag<'_>) -> Option<NodeKind> {
    Some(match tag {
        Tag::Paragraph     => NodeKind::Paragraph,
        Tag::Heading { level, .. } => NodeKind::Heading(heading_level_to_u8(*level)),
        Tag::CodeBlock(kind) => {
            let language = match kind {
                pulldown_cmark::CodeBlockKind::Indented => None,
                pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                    if lang.is_empty() { None } else { Some(lang.to_string()) }
                }
            };
            NodeKind::CodeBlock(language, String::new())
        }
        Tag::BlockQuote(_) => NodeKind::BlockQuote,
        Tag::List(Some(start)) => NodeKind::OrderedList(*start as u32),
        Tag::List(None)        => NodeKind::BulletList,
        Tag::Item              => NodeKind::Item,
        Tag::Emphasis          => NodeKind::Emphasis,
        Tag::Strong            => NodeKind::Strong,
        Tag::Link { dest_url, .. } => NodeKind::Link(dest_url.to_string()),
        // Image / FootnoteDefinition / Table / etc. — not in v0.1 WIT.
        _ => return None,
    })
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1, HeadingLevel::H2 => 2, HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4, HeadingLevel::H5 => 5, HeadingLevel::H6 => 6,
    }
}

// ── tree → WIT document ─────────────────────────────────────────────

fn collect_block(node: &Node, out: &mut Vec<Block>) {
    match &node.kind {
        NodeKind::Paragraph => {
            out.push(Block::Paragraph(collect_runs(node, &RunStyle::default())));
        }
        NodeKind::Heading(level) => {
            out.push(Block::Heading(HeadingBlock {
                level: *level,
                runs:  collect_runs(node, &RunStyle::default()),
            }));
        }
        NodeKind::CodeBlock(language, text) => {
            out.push(Block::CodeBlock(CodeBlock {
                language: language.clone(),
                text:     text.clone(),
            }));
        }
        NodeKind::ThematicBreak => out.push(Block::ThematicBreak),
        NodeKind::BulletList => {
            out.push(Block::BulletList(collect_items(node)));
        }
        NodeKind::OrderedList(start) => {
            out.push(Block::OrderedList(OrderedListBlock {
                start: *start,
                items: collect_items(node),
            }));
        }
        NodeKind::BlockQuote => {
            let mut simples: Vec<SimpleBlock> = Vec::new();
            for child in &node.children {
                collect_simple_block(child, &mut simples);
            }
            out.push(Block::BlockQuote(simples));
        }
        // Inline / unknown — likely a bare inline node that was not
        // wrapped in a Paragraph (e.g. tight-list-item children).
        // Promote to a paragraph by walking the node itself as inline.
        _ => {
            let mut runs = Vec::new();
            walk_inline(node, &RunStyle::default(), &mut runs);
            if !runs.is_empty() {
                out.push(Block::Paragraph(runs));
            }
        }
    }
}

fn collect_simple_block(node: &Node, out: &mut Vec<SimpleBlock>) {
    match &node.kind {
        NodeKind::Paragraph => {
            out.push(SimpleBlock::Paragraph(collect_runs(node, &RunStyle::default())));
        }
        NodeKind::Heading(level) => {
            out.push(SimpleBlock::Heading(HeadingBlock {
                level: *level,
                runs:  collect_runs(node, &RunStyle::default()),
            }));
        }
        NodeKind::CodeBlock(language, text) => {
            out.push(SimpleBlock::CodeBlock(CodeBlock {
                language: language.clone(),
                text:     text.clone(),
            }));
        }
        NodeKind::ThematicBreak => out.push(SimpleBlock::ThematicBreak),
        // Nested list / block-quote inside a list item — flatten to the
        // current simple-block sequence (outer marker is lost; content
        // preserved). Tradeoff documented in markdown.wit.
        NodeKind::BulletList | NodeKind::OrderedList(_) | NodeKind::BlockQuote
        | NodeKind::Item => {
            for child in &node.children {
                collect_simple_block(child, out);
            }
        }
        // Same fallthrough trick as collect_block — tight-list items
        // emit inline nodes (Text / Strong / Emphasis / …) directly
        // without a Paragraph wrapper. Walk the node itself as inline.
        _ => {
            let mut runs = Vec::new();
            walk_inline(node, &RunStyle::default(), &mut runs);
            if !runs.is_empty() {
                out.push(SimpleBlock::Paragraph(runs));
            }
        }
    }
}

fn collect_items(list_node: &Node) -> Vec<ListItem> {
    let mut items: Vec<ListItem> = Vec::new();
    for item in &list_node.children {
        if !matches!(item.kind, NodeKind::Item) {
            continue;
        }
        let mut blocks: Vec<SimpleBlock> = Vec::new();
        // Tight-list items emit inline children directly (no Paragraph
        // wrapper) — e.g. `Text("The ")`, `Strong { Text("installer") }`,
        // `Text(" copied ")`, … as siblings of Item. Coalesce consecutive
        // inlines into ONE SimpleBlock::Paragraph; flush + recurse only
        // when a real block child appears. Without this, each inline
        // segment becomes its own paragraph and renders as a stack of
        // one-word lines on the consumer side.
        let mut pending: Vec<Run> = Vec::new();
        for child in &item.children {
            if is_block_kind(&child.kind) {
                if !pending.is_empty() {
                    blocks.push(SimpleBlock::Paragraph(std::mem::take(&mut pending)));
                }
                collect_simple_block(child, &mut blocks);
            } else {
                walk_inline(child, &RunStyle::default(), &mut pending);
            }
        }
        if !pending.is_empty() {
            blocks.push(SimpleBlock::Paragraph(pending));
        }
        items.push(ListItem { blocks });
    }
    items
}

/// True for any pulldown-cmark node that should produce its own
/// SimpleBlock (Paragraph/Heading/CodeBlock/etc.). False for inline
/// nodes that should accumulate into the surrounding paragraph.
fn is_block_kind(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Paragraph
        | NodeKind::Heading(_)
        | NodeKind::CodeBlock(_, _)
        | NodeKind::ThematicBreak
        | NodeKind::BulletList
        | NodeKind::OrderedList(_)
        | NodeKind::BlockQuote
        | NodeKind::Item
    )
}

#[derive(Default, Clone)]
struct RunStyle {
    styles:   Vec<InlineStyle>,
    link_url: Option<String>,
}

impl RunStyle {
    fn with(&self, s: InlineStyle) -> Self {
        let mut next = self.clone();
        next.styles.push(s);
        next
    }
    fn with_link(&self, url: String) -> Self {
        let mut next = self.clone();
        next.link_url = Some(url);
        next
    }
    fn run(&self, text: String) -> Run {
        Run { text, styles: self.styles.clone(), link_url: self.link_url.clone() }
    }
}

fn collect_runs(parent: &Node, style: &RunStyle) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for child in &parent.children {
        walk_inline(child, style, &mut runs);
    }
    runs
}

fn walk_inline(node: &Node, style: &RunStyle, out: &mut Vec<Run>) {
    match &node.kind {
        NodeKind::Text(t) => out.push(style.run(t.clone())),
        NodeKind::SoftBreak => out.push(style.run(" ".to_string())),
        NodeKind::HardBreak => out.push(style.run("\n".to_string())),
        NodeKind::InlineCode(c) => {
            out.push(style.with(InlineStyle::Code).run(c.clone()));
        }
        NodeKind::Emphasis => {
            let next = style.with(InlineStyle::Emphasis);
            for child in &node.children {
                walk_inline(child, &next, out);
            }
        }
        NodeKind::Strong => {
            let next = style.with(InlineStyle::Strong);
            for child in &node.children {
                walk_inline(child, &next, out);
            }
        }
        NodeKind::Link(url) => {
            let next = style.with_link(url.clone());
            for child in &node.children {
                walk_inline(child, &next, out);
            }
        }
        _ => {
            for child in &node.children {
                walk_inline(child, style, out);
            }
        }
    }
}
