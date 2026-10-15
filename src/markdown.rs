//! Markdown extensions shared by the explanation and note previews.

use gpui_kit::base::{
    InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode, MarkdownParseContext,
    MarkdownPlugin, markdown_ast,
};
use gpui_kit::{App, Image, ImageFormat, IntoElement, Window, div, img, prelude::*, px};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_svg::{SvgColorSyntax, SvgOptions, render_to_svg_with_color_syntax};
use ratex_types::{Color, MathStyle};
use std::sync::{Arc, OnceLock};

struct Formula {
    image: Arc<Image>,
    width: f32,
    height: f32,
    baseline: f32,
}

fn render_formula(source: &str, block: bool) -> Option<Formula> {
    // Do not turn malformed TeX into an empty image. The Markdown renderer
    // will keep the original source visible when this returns None.
    if source.trim().is_empty() || source.len() > 16_384 {
        return None;
    }
    let nodes = ratex_parser::parse(source).ok()?;
    let options = LayoutOptions {
        style: if block {
            MathStyle::Display
        } else {
            MathStyle::Text
        },
        color: Color::rgb(0.94, 0.96, 0.98),
        ..Default::default()
    };
    let display = to_display_list(&layout(&nodes, &options));
    let font_size = if block { 22.0 } else { 17.0 };
    let padding = if block { 8.0 } else { 2.0 };
    let svg = render_to_svg_with_color_syntax(
        &display,
        &SvgOptions {
            font_size,
            padding,
            embed_glyphs: true,
            ..Default::default()
        },
        SvgColorSyntax::Rgb,
    );
    Some(Formula {
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())),
        width: (display.width * font_size + padding * 2.0).max(1.0) as f32,
        height: (display.total_height() * font_size + padding * 2.0).max(1.0) as f32,
        baseline: (display.height * font_size + padding) as f32,
    })
}

struct FormulaPlugin {
    block: bool,
}

impl MarkdownPlugin for FormulaPlugin {
    fn is_block(&self) -> bool {
        self.block
    }

    fn name(&self) -> &str {
        if self.block {
            "leetdesk-math-block"
        } else {
            "leetdesk-math-inline"
        }
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let source = match (self.block, node) {
            (true, markdown_ast::Node::Math(math)) => &math.value,
            (false, markdown_ast::Node::InlineMath(math)) => &math.value,
            _ => return None,
        };
        let formula = render_formula(source, self.block)?;
        let markdown = cx.node_source(node).unwrap_or(source).to_string();
        Some(
            MarkdownNode::new(self.name(), formula)
                .text(markdown.clone())
                .markdown(markdown),
        )
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, _: &mut App) -> impl IntoElement {
        let formula = node.data::<Formula>().expect("formula node has image");
        div().w_full().py_2().flex().justify_center().child(
            img(formula.image.clone())
                .w(px(formula.width))
                .h(px(formula.height)),
        )
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        _: &InlineRenderContext,
        _: &mut Window,
        _: &mut App,
    ) -> Option<InlineElement> {
        let formula = node.data::<Formula>()?;
        Some(
            InlineElement::new(
                img(formula.image.clone())
                    .w(px(formula.width))
                    .h(px(formula.height)),
            )
            .with_baseline(px(formula.baseline)),
        )
    }
}

pub fn extensions() -> MarkdownExtensions {
    static EXTENSIONS: OnceLock<MarkdownExtensions> = OnceLock::new();
    EXTENSIONS
        .get_or_init(|| {
            MarkdownExtensions::default()
                .plugin(FormulaPlugin { block: false })
                .plugin(FormulaPlugin { block: true })
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::render_formula;

    #[test]
    fn renders_inline_and_display_math_as_images() {
        let inline = render_formula(r"O(n^2)", false).unwrap();
        let display = render_formula(r"\sum_{i=1}^{n} \frac{1}{i}", true).unwrap();
        assert!(inline.width > 0.0 && inline.height > 0.0);
        assert!(display.width > inline.width);
        assert!(display.height > inline.height);
        let svg = std::str::from_utf8(display.image.bytes()).unwrap();
        assert!(svg.contains("<svg") && svg.contains("<path"));
        assert!(!svg.contains("font-family="), "公式 SVG 必须自带字形");
        assert!(render_formula(r"\notacommand", false).is_none());
    }
}
