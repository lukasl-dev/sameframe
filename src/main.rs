use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    font::fontsource::fontsource_font,
    router::{RouterBuilderDiscoverExt, module_router, page},
    view::{View, view},
};

#[tokio::main]
async fn main() -> Result<()> {
    let router = module_router!()
        .assets(AssetBundle::load()?)
        .discover()
        .build();

    topcoat::start(router).await?;
    Ok(())
}

#[page]
async fn home() -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"Sameframe"</title>
                topcoat::font::link(
                    font: fontsource_font!(GEIST, weight: [400, 700], style: Normal)
                )
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                topcoat::dev::script()
            </head>
            <body></body>
        </html>
    })
}
