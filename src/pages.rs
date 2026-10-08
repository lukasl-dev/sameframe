use topcoat::{
    Result,
    asset::asset,
    context::{Cx, app_context},
    router::{href, page, path_param},
    view::{View, attributes, component, view},
};

use crate::components::button::{ButtonVariant, button};
use crate::rooms::Rooms;

path_param!(room_id);

#[component]
async fn header() -> Result<impl View> {
    Ok(view! {
        <header
            class="mx-auto flex min-h-16 w-[calc(100%-32px)] max-w-[1040px] items-center justify-between gap-4 border-b border-border/70 sm:w-[calc(100%-48px)]"
        >
            <a
                class="flex min-h-11 items-center gap-2.5 text-sm font-semibold tracking-tight"
                href=(href!(home))
                aria-label="Sameframe home"
            >
                <svg
                    class="size-7 text-primary"
                    viewBox="0 0 32 32"
                    fill="none"
                    aria-hidden="true"
                >
                    <rect
                        x="3"
                        y="4"
                        width="23"
                        height="19"
                        rx="5"
                        stroke="currentColor"
                        stroke-width="1.5"
                    ></rect>
                    <rect
                        x="7"
                        y="9"
                        width="23"
                        height="19"
                        rx="5"
                        fill="var(--mantle)"
                        stroke="currentColor"
                        stroke-width="1.5"
                    ></rect>
                    <path d="M16 15L22 18.5L16 22V15Z" fill="currentColor"></path>
                </svg>
                <span>"sameframe"</span>
            </a>
            <span class="hidden text-[11px] text-muted-foreground sm:block">
                "Good company. Same frame."
            </span>
        </header>
    })
}

#[component]
async fn viewing_nook(#[default] quiet: bool) -> Result<impl View> {
    Ok(view! {
        <svg
            class="viewing-nook mx-auto h-auto w-full max-w-[430px]"
            viewBox="0 0 430 245"
            fill="none"
            aria-hidden="true"
        >
            // A small, self-contained illustration: no image/font requests or animation.
            <ellipse
                cx="215"
                cy="220"
                rx="166"
                ry="13"
                fill="#313244"
                fill-opacity=".35"
            ></ellipse>
            <path d="M48 216H380" stroke="#45475a" stroke-linecap="round"></path>
            <path
                d="M62 216V111M43 112H81L71 82H53L43 112Z"
                stroke="#fab387"
                stroke-opacity=".65"
                stroke-width="2"
                stroke-linejoin="round"
            ></path>
            <path
                d="M50 215H74"
                stroke="#fab387"
                stroke-opacity=".65"
                stroke-width="2"
                stroke-linecap="round"
            ></path>
            <path
                class="nook-glow"
                d="M46 119L26 177H98L78 119"
                fill="#fab387"
                fill-opacity=".08"
            ></path>
            <rect
                x="125"
                y="38"
                width="180"
                height="108"
                rx="13"
                fill="#181825"
                stroke="#45475a"
                stroke-width="2"
            ></rect>
            <rect x="134" y="47" width="162" height="90" rx="7" fill="#11111b"></rect>
            if quiet {
                <path
                    d="M204 87C205 78 218 77 222 84C228 84 231 89 229 94C227 99 220 99 214 99H202C194 98 195 88 204 87Z"
                    stroke="#a6adc8"
                    stroke-opacity=".55"
                    stroke-width="1.5"
                    stroke-linecap="round"
                ></path>
                <path
                    class="nook-dream"
                    d="M230 62H238L230 72H238M241 51H247L241 58H247"
                    stroke="#a6adc8"
                    stroke-opacity=".45"
                    stroke-width="1.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                ></path>
            } else {
                <circle
                    class="nook-screen"
                    cx="215"
                    cy="92"
                    r="24"
                    fill="#cba6f7"
                    fill-opacity=".12"
                ></circle>
                <path d="M210 81L226 92L210 103V81Z" fill="#cba6f7" fill-opacity=".8"></path>
                <path
                    d="M149 61H163M149 66H156"
                    stroke="#45475a"
                    stroke-linecap="round"
                ></path>
            }
            <path
                d="M204 147V158M226 147V158M195 158H235"
                stroke="#45475a"
                stroke-width="2"
                stroke-linecap="round"
            ></path>
            <rect x="103" y="165" width="222" height="14" rx="4" fill="#313244"></rect>
            <path
                d="M112 179V212M315 179V212"
                stroke="#45475a"
                stroke-width="3"
                stroke-linecap="round"
            ></path>
            <rect
                x="137"
                y="162"
                width="23"
                height="3"
                rx="1.5"
                fill="#94e2d5"
                fill-opacity=".5"
            ></rect>
            <path
                d="M280 165V153H291V163C291 164 290 165 289 165H280ZM291 155H294C299 155 299 161 294 161H291"
                stroke="#fab387"
                stroke-opacity=".75"
                stroke-width="1.5"
                stroke-linejoin="round"
            ></path>
            <path
                class="nook-steam"
                d="M282 147C279 143 286 141 283 137M288 147C285 143 292 141 289 137"
                stroke="#fab387"
                stroke-opacity=".3"
                stroke-linecap="round"
            ></path>
            <path
                d="M337 177V145M337 161C325 160 323 152 324 148C333 149 337 154 337 161ZM338 154C349 153 353 144 351 139C342 141 338 147 338 154Z"
                stroke="#a6e3a1"
                stroke-opacity=".55"
                stroke-width="1.5"
                stroke-linejoin="round"
            ></path>
            <path d="M327 177H348L344 194H331L327 177Z" fill="#313244" stroke="#45475a"></path>
            <rect
                x="140"
                y="192"
                width="148"
                height="27"
                rx="9"
                fill="#313244"
                stroke="#45475a"
                stroke-width="1.5"
            ></rect>
            <path
                d="M149 194V183C149 177 154 175 159 175H269C274 175 279 178 279 183V194"
                fill="#313244"
                stroke="#45475a"
                stroke-width="1.5"
            ></path>
            <path
                d="M160 200H267M213 178V197M147 220V224M282 220V224"
                stroke="#45475a"
                stroke-linecap="round"
            ></path>
            <rect
                x="161"
                y="182"
                width="25"
                height="18"
                rx="5"
                transform="rotate(-8 161 182)"
                fill="#cba6f7"
                fill-opacity=".23"
            ></rect>
            <path
                d="M245 179H271L275 206H253L245 179Z"
                fill="#94e2d5"
                fill-opacity=".15"
            ></path>
            <path
                d="M251 187L273 187M253 194H274"
                stroke="#94e2d5"
                stroke-opacity=".18"
            ></path>
            <path
                d="M357 68V78M352 73H362M103 53V59M100 56H106"
                stroke="#fab387"
                stroke-opacity=".45"
                stroke-linecap="round"
            ></path>
            <circle cx="88" cy="151" r="2" fill="#cba6f7" fill-opacity=".3"></circle>
            <circle cx="334" cy="97" r="2" fill="#94e2d5" fill-opacity=".35"></circle>
        </svg>
    })
}

#[component]
async fn footer() -> Result<impl View> {
    Ok(view! {
        <footer
            class="mx-auto flex w-[calc(100%-32px)] max-w-[1040px] flex-wrap justify-between gap-2 border-t border-border/50 py-5 text-[11px] text-muted-foreground sm:w-[calc(100%-48px)]"
        >
            <span>"A little closer, wherever you are."</span>
            <span>"No accounts. Just your people."</span>
        </footer>
    })
}

#[page("/")]
pub(crate) async fn home() -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" class="dark">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <meta name="color-scheme" content="dark">
                <meta name="theme-color" content="#1e1e2e">
                <meta
                    name="description"
                    content="Create a room, share a link, and watch YouTube together in sync. No accounts, just a shared moment."
                >
                <title>"Sameframe — watch together"</title>
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                <script type="module" src=(asset!("assets/home.js"))></script>
                topcoat::dev::script()
            </head>
            <body data-sync-url=(asset!("assets/sync.js"))>
                <a class="skip-link" href="#main">"Skip to content"</a>
                header()
                <main
                    id="main"
                    class="mx-auto grid w-[calc(100%-32px)] max-w-[940px] flex-1 content-center gap-7 py-8 sm:w-[calc(100%-48px)] sm:py-14 min-[800px]:grid-cols-[minmax(0,1fr)_340px] min-[800px]:items-center min-[800px]:gap-x-14 min-[800px]:gap-y-6"
                >
                    <section class="min-w-0" aria-labelledby="welcome-title">
                        <p class="mb-4 text-[11px] tracking-[.16em] text-[#fab387]/80">
                            "A LITTLE ROOM FOR YOUR PEOPLE"
                        </p>
                        <h1
                            id="welcome-title"
                            class="text-[clamp(2rem,6vw,3.75rem)] font-medium leading-[1.08] tracking-[-.05em]"
                        >
                            "Pull up a seat."
                            <br>
                            <span class="text-primary/85">"Press play together."</span>
                        </h1>
                        <p
                            class="mt-5 max-w-[400px] text-sm leading-relaxed text-muted-foreground"
                        >
                            "The next rabbit hole. An old favourite. One more song. A shared screen, a little chat, and your favourite people — wherever they are."
                        </p>
                    </section>
                    <section
                        class="min-w-0 rounded-[20px] border border-border bg-card p-5 shadow-sm sm:p-6 min-[800px]:col-start-2 min-[800px]:row-span-2 min-[800px]:row-start-1"
                        aria-labelledby="start-title"
                    >
                        <span
                            class="mb-4 inline-flex size-10 items-center justify-center rounded-xl bg-primary/10 text-lg text-primary"
                            aria-hidden="true"
                        >
                            "✳"
                        </span>
                        <h2 id="start-title" class="text-lg font-medium">
                            "Make yourself at home"
                        </h2>
                        <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
                            "Start a room, pick something good, and send your people the link."
                        </p>
                        button(
                            attrs: attributes! { id="create-room" class="mt-6 h-11 w-full" type="button" },
                            <span aria-hidden="true">"+"</span>
                            "Create a room"
                        )
                        <div
                            class="my-6 flex items-center gap-3 text-[11px] text-muted-foreground"
                        >
                            <span class="h-px flex-1 bg-border"></span>
                            "or settle into theirs"
                            <span class="h-px flex-1 bg-border"></span>
                        </div>
                        <form id="join-form">
                            <label for="join-link" class="mb-2 text-xs font-medium">
                                "Got an invite?"
                            </label>
                            <input
                                id="join-link"
                                class="border-border bg-background px-3 py-2 sm:text-sm"
                                name="room"
                                type="text"
                                placeholder="Paste a room link or code"
                                required=(true)
                                autocomplete="off"
                                autocapitalize="none"
                                spellcheck="false"
                                aria-describedby="join-help home-error"
                            >
                            button(
                                variant: ButtonVariant::Secondary,
                                attrs: attributes! { class="mt-2.5 h-11 w-full" type="submit" },
                                "Join room"
                            )
                            <p
                                id="join-help"
                                class="mt-3 text-[11px] leading-relaxed text-muted-foreground"
                            >
                                "No sign-up, no fuss. Just come as you are."
                            </p>
                        </form>
                        <p
                            id="home-error"
                            class="mt-4 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-xs text-destructive"
                            role="alert"
                            hidden=(true)
                        ></p>
                        <noscript>
                            <p class="mt-4 text-xs text-destructive">
                                "Enable JavaScript to create or join a room and watch together."
                            </p>
                        </noscript>
                    </section>
                    <div class="min-[800px]:col-start-1 min-[800px]:row-start-2">
                        viewing_nook()
                        <p
                            class="mt-2 text-center text-[11px] text-muted-foreground/75"
                        >
                            "Bring a video. We’ll save you a spot."
                        </p>
                    </div>
                </main>
                footer()
            </body>
        </html>
    })
}

#[page("/room/{room_id}")]
pub(crate) async fn room(cx: &Cx) -> Result<impl View> {
    let room_id = path_param::<RoomId>(cx);
    let available = app_context::<Rooms>(cx).get(room_id).is_some();

    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" class="dark">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <meta name="color-scheme" content="dark">
                <meta name="theme-color" content="#1e1e2e">
                <title>
                    (if available {
                        "Your room — Sameframe"
                    } else {
                        "Room unavailable — Sameframe"
                    })
                </title>
                <link rel="stylesheet" href=(topcoat::tailwind::stylesheet!())>
                if available {
                    <script type="module" src=(asset!("assets/room.js"))></script>
                } else {
                    <script type="module" src=(asset!("assets/home.js"))></script>
                }
                topcoat::dev::script()
            </head>
            <body
                class="room-page bg-background"
                data-room-id=(if available { Some(room_id) } else { None })
                data-sync-url=(asset!("assets/sync.js"))
            >
                <a class="skip-link" href="#main">"Skip to content"</a>
                if available {
                    <main
                        id="main"
                        class="mx-auto w-[calc(100%-24px)] max-w-[1480px] pb-5 sm:w-[calc(100%-40px)] min-[761px]:max-w-[calc(min(1140px,max(300px,(100svh-400px)*16/9))+344px)]"
                    >
                        <div
                            class="mb-3 flex min-h-[58px] items-center justify-between gap-3 border-b border-border sm:mb-[18px] sm:min-h-16"
                        >
                            <h1 class="sr-only">"Watch together"</h1>
                            <div class="flex min-w-0 items-center gap-3">
                                <a
                                    class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
                                    href=(href!(home))
                                    aria-label="Sameframe home"
                                >
                                    "▸"
                                </a>
                                <div class="min-w-0 leading-tight">
                                    <h2 id="room-name" class="truncate text-sm font-medium">
                                        "Your viewing room"
                                    </h2>
                                    <span class="text-[10px] text-muted-foreground">
                                        "sameframe · make yourself at home"
                                    </span>
                                </div>
                            </div>
                            <div class="flex shrink-0 items-center gap-1.5">
                                <span
                                    id="connection-status"
                                    class="hidden items-center gap-1.5 px-2 text-[11px] text-muted-foreground sm:inline-flex"
                                    role="status"
                                    aria-live="polite"
                                >
                                    "Connecting…"
                                </span>
                                <button
                                    id="copy-link"
                                    class="flex min-h-10 items-center gap-2 rounded-lg px-2.5 text-xs text-foreground/80 hover:bg-foreground/5 [&_svg]:size-[18px]"
                                    type="button"
                                    aria-label="Copy room link"
                                    title="Invite your people"
                                >
                                    <svg
                                        id="copy-icon"
                                        viewBox="0 0 20 20"
                                        fill="none"
                                        aria-hidden="true"
                                    >
                                        <rect
                                            x="7"
                                            y="7"
                                            width="10"
                                            height="10"
                                            rx="2"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                        ></rect>
                                        <path
                                            d="M12 4V3H3V12H4"
                                            stroke="currentColor"
                                            stroke-width="1.5"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                        ></path>
                                    </svg>
                                    <span>"Invite"</span>
                                    <svg
                                        id="copy-success"
                                        viewBox="0 0 20 20"
                                        fill="none"
                                        aria-hidden="true"
                                        hidden=(true)
                                    >
                                        <path
                                            d="M4 10L8 14L16 6"
                                            stroke="currentColor"
                                            stroke-width="1.7"
                                            stroke-linecap="round"
                                            stroke-linejoin="round"
                                        ></path>
                                    </svg>
                                </button>
                                <span
                                    id="copy-feedback"
                                    class="sr-only"
                                    role="status"
                                    aria-live="polite"
                                ></span>
                            </div>
                        </div>
                        <div
                            class="grid items-start gap-[22px] min-[761px]:grid-cols-[minmax(0,1fr)_290px] min-[1001px]:grid-cols-[minmax(0,1fr)_320px] min-[1001px]:gap-6"
                        >
                            <div class="min-w-0">
                                <section
                                    id="host-controls"
                                    class="mb-3.5"
                                    aria-label="Choose or suggest a video"
                                >
                                    <form id="video-form" class="flex items-stretch gap-2">
                                        <label for="video-url" class="sr-only">
                                            "YouTube link"
                                        </label>
                                        <input
                                            id="video-url"
                                            class="min-w-0 flex-1 border-border bg-card px-3 py-2 sm:text-sm"
                                            name="video"
                                            type="text"
                                            required=(true)
                                            placeholder="What shall we watch? Paste a YouTube link…"
                                            autocomplete="off"
                                            autocapitalize="none"
                                            spellcheck="false"
                                            aria-describedby="playback-help"
                                        >
                                        button(
                                            attrs: attributes! {
                                                id="load-video"
                                                class="h-auto min-h-[46px] min-w-[72px]"
                                                type="submit"
                                            },
                                            "Watch"
                                        )
                                    </form>
                                    <p id="playback-help" class="sr-only">
                                        "Pick a video, or suggest one to the room."
                                    </p>
                                </section>
                                <section class="min-w-0" aria-label="Shared video player">
                                    <div id="player-shell" class="min-w-0">
                                        <div
                                            class="relative isolate aspect-video overflow-hidden rounded-xl border border-border bg-[#11111b] [&_iframe]:absolute [&_iframe]:inset-0 [&_iframe]:size-full [&_iframe]:border-0"
                                        >
                                            <p
                                                id="room-error"
                                                class="absolute inset-x-3 top-3 z-20 rounded-lg border border-destructive/30 bg-card/95 px-3 py-2 text-xs text-destructive"
                                                role="alert"
                                                hidden=(true)
                                            ></p>
                                            <div
                                                id="player-placeholder"
                                                class="absolute inset-0 z-10 flex flex-col items-center justify-center bg-[#11111b] p-4 text-center"
                                            >
                                                <svg
                                                    class="mb-2.5 size-8 text-primary opacity-55 sm:mb-5 sm:size-12"
                                                    viewBox="0 0 48 48"
                                                    fill="none"
                                                    aria-hidden="true"
                                                >
                                                    <rect
                                                        x="4"
                                                        y="8"
                                                        width="40"
                                                        height="30"
                                                        rx="7"
                                                        stroke="currentColor"
                                                        stroke-width="1.5"
                                                    ></rect>
                                                    <path d="M20 17L31 23L20 29V17Z" fill="currentColor"></path>
                                                </svg>
                                                <span
                                                    class="mb-3 hidden text-[10px] tracking-[.13em] text-muted-foreground sm:block"
                                                >
                                                    "MAKE YOURSELF AT HOME"
                                                </span>
                                                <h2 class="text-sm font-medium sm:text-lg">
                                                    "Good company. Something good to watch."
                                                </h2>
                                                <p
                                                    class="mt-2 max-w-[370px] text-[11px] text-muted-foreground sm:text-xs"
                                                >
                                                    "Pick a video above, or leave a suggestion for your people."
                                                </p>
                                            </div>
                                            <div id="player-mount" class="absolute inset-0 size-full"></div>
                                        </div>
                                        <div
                                            class="relative flex h-12 items-center gap-3 text-[11px] text-muted-foreground"
                                        >
                                            <span id="playback-status" class="min-w-0 flex-1 truncate">
                                                "YouTube · waiting for a video"
                                            </span>
                                            <span
                                                id="room-notice"
                                                class="min-w-0 flex-1 truncate text-right"
                                                role="status"
                                                aria-live="polite"
                                                hidden=(true)
                                            ></span>
                                            <button
                                                id="join-playback"
                                                class="ml-auto min-h-10 shrink-0 rounded-lg border border-border bg-popover px-3 text-[11px] text-foreground"
                                                type="button"
                                                hidden=(true)
                                            >
                                                "Join playback on this device"
                                            </button>
                                            <button
                                                id="reconnect-button"
                                                class="ml-auto min-h-10 shrink-0 rounded-lg border border-border bg-popover px-3 text-[11px] text-foreground"
                                                type="button"
                                                hidden=(true)
                                            >
                                                "Reconnect to room"
                                            </button>
                                        </div>
                                    </div>
                                    <p id="guest-note" class="sr-only" hidden=(true)>
                                        "Your player controls are local. Rejoin to catch up with your people."
                                    </p>
                                    <noscript>
                                        <p class="message message-error">
                                            "Enable JavaScript to connect to this room and watch in sync."
                                        </p>
                                    </noscript>
                                </section>
                                <section
                                    class="mt-4 border-t border-border pt-3.5"
                                    aria-labelledby="company-title"
                                >
                                    <div class="mb-3 flex items-center justify-between gap-3">
                                        <h2 id="company-title" class="text-[13px] font-medium">
                                            "In good company"
                                            <span
                                                id="member-count"
                                                class="ml-1 text-[11px] text-muted-foreground"
                                            >
                                                "—"
                                            </span>
                                        </h2>
                                        <span
                                            id="role-label"
                                            class="text-[11px] text-muted-foreground"
                                        >
                                            "Connecting…"
                                        </span>
                                    </div>
                                    <ul
                                        id="members"
                                        class="m-0 flex max-h-[112px] list-none flex-wrap gap-2 overflow-y-auto p-0 [scrollbar-width:thin]"
                                        aria-label="Room members"
                                    ></ul>
                                    <p
                                        class="mt-3 text-[10px] leading-relaxed text-muted-foreground"
                                    >
                                        "Keeper picks the programme · Co-keepers lend a hand · Companions bring the company"
                                    </p>
                                </section>
                            </div>
                            <aside
                                class="relative flex h-[clamp(360px,60svh,540px)] min-w-0 flex-col overflow-hidden rounded-[14px] border border-border bg-card min-[761px]:h-[clamp(380px,calc(100svh-112px),760px)]"
                                aria-labelledby="chat-title"
                            >
                                <div
                                    class="flex items-center justify-between border-b border-border/70 px-[18px] py-4"
                                >
                                    <div>
                                        <h2 id="chat-title" class="text-[15px] font-medium">
                                            "The fireside"
                                        </h2>
                                        <p class="mt-1 text-[11px] text-muted-foreground">
                                            "A little conversation on the side."
                                        </p>
                                    </div>
                                    <span
                                        class="text-2xl text-[#fab387]/70"
                                        aria-hidden="true"
                                    >
                                        "✳"
                                    </span>
                                </div>
                                <div
                                    id="chat-feed"
                                    class="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-[18px] [scrollbar-width:thin] [scrollbar-color:var(--surface0)_transparent]"
                                    role="log"
                                    aria-label="Messages and room activity"
                                    aria-live="polite"
                                    aria-relevant="additions"
                                    tabindex="0"
                                ></div>
                                <button
                                    id="chat-unread"
                                    class="absolute bottom-[98px] self-center rounded-full border border-border bg-popover px-3 py-2 text-[11px] shadow-sm"
                                    type="button"
                                    hidden=(true)
                                >
                                    "New messages ↓"
                                </button>
                                <p
                                    id="chat-error"
                                    class="px-3.5 py-2 text-xs text-destructive"
                                    role="alert"
                                    hidden=(true)
                                ></p>
                                <form
                                    id="chat-form"
                                    class="flex gap-2 border-t border-border px-3 pt-3"
                                >
                                    <label for="chat-message" class="sr-only">
                                        "Message your people"
                                    </label>
                                    <input
                                        id="chat-message"
                                        class="border-border bg-background px-3 py-2 sm:text-sm"
                                        name="message"
                                        type="text"
                                        maxlength="500"
                                        required=(true)
                                        placeholder="Say something nice…"
                                        autocomplete="off"
                                        aria-describedby="chat-help"
                                    >
                                    button(
                                        variant: ButtonVariant::Secondary,
                                        attrs: attributes! {
                                            id="send-message"
                                            class="h-auto min-h-[46px] w-[42px] px-0 text-xl"
                                            type="submit"
                                            aria-label="Send message"
                                        },
                                        "↑"
                                    )
                                </form>
                                <p
                                    id="chat-help"
                                    class="px-3.5 pt-2 pb-3 text-[10px] text-muted-foreground"
                                >
                                    "YouTube links become video suggestions."
                                </p>
                            </aside>
                        </div>
                    </main>
                } else {
                    header()
                    <main
                        id="main"
                        class="mx-auto grid w-[calc(100%-32px)] max-w-[520px] flex-1 content-center py-10 text-center sm:py-14"
                    >
                        <section aria-labelledby="unavailable-title">
                            <div class="mx-auto mb-6 max-w-[360px]">
                                viewing_nook(quiet: true)
                            </div>
                            <p
                                class="mb-3 text-[10px] tracking-[.16em] text-[#fab387]/80"
                            >
                                "ROOM UNAVAILABLE · KETTLE STILL ON"
                            </p>
                            <h1
                                id="unavailable-title"
                                class="text-[clamp(2rem,5vw,2.75rem)] font-medium"
                            >
                                "This room has gone quiet."
                            </h1>
                            <p
                                class="mx-auto mt-4 max-w-[370px] text-sm leading-relaxed text-muted-foreground"
                            >
                                "The room may have expired, or the invite might be a little off. Check the link with your people — or make a fresh room for the next one."
                            </p>
                            <div
                                class="mt-7 flex flex-wrap items-center justify-center gap-2"
                            >
                                button(
                                    attrs: attributes! { id="create-room" class="h-11" type="button" },
                                    "Create a new room"
                                )
                                <a
                                    class="inline-flex min-h-11 items-center justify-center rounded-lg px-4 text-sm text-muted-foreground hover:bg-foreground/5 hover:text-foreground"
                                    href=(href!(home))
                                >
                                    "Back to home"
                                </a>
                            </div>
                            <p
                                id="home-error"
                                class="mx-auto mt-4 max-w-[370px] rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-xs text-destructive"
                                role="alert"
                                hidden=(true)
                            ></p>
                            <noscript>
                                <p class="mt-4 text-xs text-muted-foreground">
                                    "Enable JavaScript to start a fresh room, or head back home."
                                </p>
                            </noscript>
                        </section>
                    </main>
                    footer()
                }
            </body>
        </html>
    })
}
