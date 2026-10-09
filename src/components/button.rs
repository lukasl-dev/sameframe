use topcoat::{
    Result,
    view::{Attributes, Child, StaticClass, View, class, component, view},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Outline,
    Ghost,
    Destructive,
}

impl ButtonVariant {
    fn classes(self) -> StaticClass {
        match self {
            Self::Primary => class!(
                "border-transparent bg-primary text-primary-foreground shadow-xs \
                 hover:bg-primary/90 active:bg-primary/80",
            ),
            Self::Secondary => class!(
                "border-transparent bg-foreground/5 text-foreground shadow-xs \
                 hover:bg-foreground/10 active:bg-foreground/15",
            ),
            Self::Outline => class!(
                "border-border text-foreground hover:bg-foreground/5 \
                 active:bg-foreground/10",
            ),
            Self::Ghost => class!(
                "border-transparent text-foreground hover:bg-foreground/5 active:bg-foreground/10",
            ),
            Self::Destructive => class!(
                "border-transparent bg-destructive text-destructive-foreground shadow-xs \
                 hover:bg-destructive/90 active:bg-destructive/80",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
    Icon,
}

impl ButtonSize {
    fn classes(self) -> StaticClass {
        match self {
            Self::Sm => class!("h-8 gap-1.5 rounded-md px-3"),
            Self::Md => class!("h-9 gap-2 rounded-lg px-4"),
            Self::Lg => class!("h-10 gap-2 rounded-lg px-5"),
            Self::Icon => class!("size-9 rounded-lg"),
        }
    }
}

const BASE: StaticClass = class!(
    "inline-flex shrink-0 items-center justify-center border \
     text-sm font-medium whitespace-nowrap transition-colors outline-none select-none \
     focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 \
     focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50",
);

#[component]
pub async fn button(
    #[default] variant: ButtonVariant,
    #[default] size: ButtonSize,
    #[default] mut attrs: Attributes,
    #[default] child: Child<'_>,
) -> Result<impl View> {
    Ok(view! {
        <button
            class=(class!(
                BASE,
                variant.classes(),
                size.classes(),
                attrs.remove("class"),
            ))
            (attrs)
        >
            (child)
        </button>
    })
}
