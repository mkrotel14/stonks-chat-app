//! Offline actual native shaping/wrapping/emoji/spoiler fixture; no network.
use gpui_kit::*;
use std::{collections::HashSet, rc::Rc, sync::Arc};
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    markdown::{self, InlineActions, InlineEmoji, InlineMedia, MentionContext, RenderOptions},
    Theme,
};
struct Preview {
    revealed: HashSet<String>,
    status: String,
}
#[derive(IntoElement)]
struct AdvanceProbe;
#[derive(IntoElement)]
struct MixedPositionProbe(u8);
impl RenderOnce for MixedPositionProbe {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let mut value = String::new();
        let mut objects = Vec::new();
        let mut overrides = Vec::new();
        for (index, width) in [20., 32., 20.].into_iter().enumerate() {
            match self.0 {
                0 => {
                    if index > 0 {
                        value.push(' ');
                    }
                }
                1 => value.push_str(if index == 0 { "before " } else { " between " }),
                _ => {}
            }
            let units = (width / 15.5 * 2048.0_f32).round() as u32;
            let glyph = char::from_u32(0xDFFF + units).unwrap();
            let start = value.len();
            value.push(glyph);
            let range = start..value.len();
            overrides.push((range.clone(), fonts::INLINE_OBJECTS.into()));
            objects.push((range, width));
        }
        let runs = markdown::inline_text_runs(&window.text_style(), value.len(), &[], &overrides);
        let text = StyledText::new(value).with_runs(runs);
        let layout = text.layout().clone();
        div()
            .relative()
            .w(px(430.))
            .text_size(px(15.5))
            .line_height(px(32.))
            .child(text)
            .child(
                canvas(
                    move |_, _, _| {
                        for (index, (range, width)) in objects.into_iter().enumerate() {
                            let start = layout.position_for_index(range.start).unwrap();
                            let end = layout.position_for_index(range.end).unwrap();
                            let inside = layout.position_for_index(range.start + 1).unwrap();
                            assert!(
                                (f32::from(end.x - start.x) - width).abs() < 0.03,
                                "mixed native glyph{index} wrongadvance: expected{width} actual{}",
                                f32::from(end.x - start.x)
                            );
                            assert!(
                                (f32::from(inside.x - start.x) - width).abs() < 0.03,
                                "mixed native glyph{index} anchor differs from reservedadvance"
                            );
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}
impl RenderOnce for AdvanceProbe {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        for font_size in [12., 15.5, 16.74, 19.375, 22.475] {
            for desired in [20., 32., font_size * 1.57] {
                let units = (desired / font_size * 2048.0_f32).round() as u32;
                let glyph = char::from_u32(0xDFFF + units).unwrap().to_string();
                for weight in [FontWeight::NORMAL, FontWeight(800.)] {
                    let mut run = window.text_style().to_run(glyph.len());
                    run.font = font(fonts::INLINE_OBJECTS);
                    run.font.weight = weight;
                    let shaped = window.text_system().shape_line(
                        glyph.clone().into(),
                        px(font_size),
                        &[run],
                        None,
                    );
                    assert!(
                        (f32::from(shaped.width()) - desired).abs() < 0.03,
                        "native inline-object advance mismatch: expected={desired:.5}px actual={:.5}px font_size={font_size:.5}px weight={} units={units} requested_family={} shaped_fonts={:?}",
                        f32::from(shaped.width()),weight.0,fonts::INLINE_OBJECTS,
                        shaped.runs.iter().map(|run|(run.font_id,window.text_system().get_font_for_id(run.font_id).map(|font|font.family))).collect::<Vec<_>>()
                    );
                }
            }
        }
        let glyph = char::from_u32(0xDFFF + (20.0_f32 / 15.5 * 2048.).round() as u32)
            .unwrap()
            .to_string();
        let value = glyph.repeat(3);
        let text = StyledText::new(value.clone())
            .with_font_family_overrides(vec![(0..value.len(), fonts::INLINE_OBJECTS.into())]);
        let layout = text.layout().clone();
        div()
            .relative()
            .w(px(31.))
            .text_size(px(15.5))
            .line_height(relative(1.5))
            .child(text)
            .child(
                canvas(
                    move |_, _, _| {
                        let first = layout
                            .position_for_index(1)
                            .expect("first object has native position");
                        let second = layout
                            .position_for_index(5)
                            .expect("wrapped object has native position");
                        assert!(
                            second.y > first.y,
                            "objects must wrap as indivisible native glyphs"
                        );
                        assert!(
                            (f32::from(second.x - layout.bounds().origin.x) - 20.).abs() < 0.03,
                            "interior-byte caret must locate object on its own wrapped row"
                        );
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = cx.entity().downgrade();
        let hover = Rc::new(move |key: &str, visible: bool, cx: &mut App| {
            target
                .update(cx, |this, cx| {
                    let changed = if visible {
                        this.revealed.insert(key.into())
                    } else {
                        this.revealed.remove(key)
                    };
                    if changed {
                        cx.notify();
                    }
                })
                .ok();
        });
        let svg=Arc::new(Image::from_bytes(ImageFormat::Svg,b"<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'><circle cx='12' cy='12' r='11' fill='#f6cb42'/><circle cx='8' cy='9' r='2'/><circle cx='16' cy='9' r='2'/><path d='M6 15q6 7 12 0' fill='none' stroke='black' stroke-width='2'/></svg>".to_vec()));
        let media = InlineMedia {
            custom: Some(Rc::new(move |_, visible, _| {
                Some(InlineEmoji {
                    label: ":fixture:".into(),
                    image: visible.then(|| ImageSource::from(svg.clone())),
                })
            })),
        };
        div().size_full().font_family(fonts::FAMILY).flex().children([Theme::dark(),Theme::light()].into_iter().enumerate().map(|(column,theme)|{
            let mut root=div().id(ElementId::Name(format!("panel-{column}").into())).flex_1().h_full().min_w_0().p(px(20.)).bg(theme.surface).text_color(theme.text).overflow_y_scroll().flex().flex_col().gap(px(14.));
            for(index,(content,width,compact))in [
                ("Native advances + wrap assertions",430.,false),
                ("Hello 😺 next 👨‍👩‍👧‍👦 and 👍🏽 after.",430.,false),
                ("😺 👨‍👩‍👧‍👦 👍🏽",430.,false),
                ("# Heading 😺 text\n\nNarrow 😺😺 and following text wrap cleanly.",150.,false),
                ("Custom <:fixture:01M0P54M56DA883E6ZYDK00BKC> beside text.\n\n||one **bold nested** <:fixture:01M0P54M56DA883E6ZYDK00BKC> secret|| and ||separate||.",430.,false),
                ("Code `abc 😺 <script>` remains text; `long_code_without_breaks_012345678901234567890123456789` wraps in this narrow box.",200.,false),
                ("Compact 😺 text\n\n||Tab to reveal **the whole original group**||.",430.,true),
            ].into_iter().enumerate(){let id=format!("fixture-{column}-{index}");root=root.child(div().w(px(width)).max_w_full().border_1().border_color(theme.outline).child(if index==0{div().flex().flex_col().child(AdvanceProbe).child(MixedPositionProbe(0)).child(MixedPositionProbe(1)).child(MixedPositionProbe(2)).into_any_element()}else{markdown::render_with_options(content,&id,&theme,&MentionContext{users:&[],channels:&[],roles:&[]},&self.revealed,hover.clone(),InlineActions::default(),media.clone(),RenderOptions{compact}).into_any_element()}));}
            root.child(Button::new(ElementId::Name(format!("reset-{column}").into()),theme).variant(ButtonVariant::Quiet).label("Reset reveals").on_click(cx.listener(|this,_,_,cx|{this.revealed.clear();this.status="Reset".into();cx.notify();}))).child(self.status.clone())
        }))
    }
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-inline-emoji-fixture".into()),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1060.), px(980.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| Preview {
                        revealed: HashSet::new(),
                        status: String::new(),
                    })
                },
            )
            .expect("inline fixture window");
            cx.activate(true);
        });
}
