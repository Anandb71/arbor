//! Naming code in the terms of the app it builds: pages, API routes and the
//! areas people test by hand (sign in, payments, the database).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Area {
    SignIn,
    Payments,
    Database,
    Emails,
    Uploads,
    Admin,
    Settings,
}

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::SignIn => "Sign in",
            Area::Payments => "Payments",
            Area::Database => "Database",
            Area::Emails => "Emails",
            Area::Uploads => "File uploads",
            Area::Admin => "Admin",
            Area::Settings => "App settings",
        }
    }

    /// The one thing to try by hand when this area may have changed.
    pub fn test(self) -> &'static str {
        match self {
            Area::SignIn => "Sign out, then sign back in",
            Area::Payments => "Run a test checkout",
            Area::Database => "Check that data still saves and loads",
            Area::Emails => "Trigger an email and check it arrives",
            Area::Uploads => "Upload a file",
            Area::Admin => "Open the admin area signed in, then signed out",
            Area::Settings => "Restart the app and check it starts cleanly",
        }
    }

    /// Changes here are worth a warning even when they look small.
    pub fn sensitive(self) -> bool {
        matches!(
            self,
            Area::SignIn | Area::Payments | Area::Database | Area::Settings
        )
    }

    const ALL: [(Area, &'static [&'static str]); 7] = [
        (
            Area::SignIn,
            &[
                "auth", "login", "logout", "signin", "signup", "session", "oauth", "password",
                "passkey", "jwt", "clerk", "nextauth",
            ],
        ),
        (
            Area::Payments,
            &[
                "payment",
                "checkout",
                "billing",
                "stripe",
                "subscription",
                "invoice",
                "razorpay",
                "paddle",
                "lemonsqueezy",
                "refund",
            ],
        ),
        (
            Area::Database,
            &[
                "migration",
                "migrations",
                "schema",
                "prisma",
                "supabase",
                "drizzle",
                "database",
                "db",
                "sql",
                "sqlx",
                "knex",
                "mongoose",
            ],
        ),
        (
            Area::Emails,
            &[
                "email", "mail", "mailer", "resend", "sendgrid", "smtp", "postmark",
            ],
        ),
        (
            Area::Uploads,
            &[
                "upload", "uploads", "storage", "bucket", "s3", "blob", "multer",
            ],
        ),
        (Area::Admin, &["admin"]),
        (
            Area::Settings,
            &[
                "env",
                "dotenv",
                "config",
                "next.config",
                "vite.config",
                "dockerfile",
                "vercel",
            ],
        ),
    ];
}

/// Lower-case words in a path or identifier.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in text.split(|c: char| !c.is_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        for token in arbor_graph::tokenize_identifier(part) {
            out.push(token.to_lowercase());
        }
        out.push(part.to_lowercase());
    }
    out
}

/// Areas a file or symbol belongs to, judged from its path and names.
pub fn areas(path: &str, names: &[String]) -> Vec<Area> {
    let lower = path.to_lowercase().replace('\\', "/");
    let file_name = lower.rsplit('/').next().unwrap_or(&lower);
    let mut tokens = words(&lower);
    for name in names {
        tokens.extend(words(name));
    }
    let mut found: Vec<Area> = Area::ALL
        .iter()
        .filter(|(area, keywords)| {
            keywords.iter().any(|keyword| tokens.iter().any(|token| token == keyword))
                // Environment files (.env, .env.local) and Django's settings.py
                // are app settings. A settings *page* is not, so the bare word
                // "settings" doesn't count.
                || (*area == Area::Settings
                    && (file_name.starts_with(".env")
                        || file_name == "dockerfile"
                        || file_name == "settings.py"))
        })
        .map(|(area, _)| *area)
        .collect();
    // "session" alone is ambiguous: a Stripe checkout session is payments.
    let session_only = !tokens.iter().any(|t| {
        [
            "auth", "login", "logout", "signin", "signup", "oauth", "password", "passkey", "jwt",
            "clerk", "nextauth",
        ]
        .contains(&t.as_str())
    });
    if session_only && found.contains(&Area::Payments) {
        found.retain(|area| *area != Area::SignIn);
    }
    found.sort();
    found.dedup();
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteKind {
    Page,
    Api,
}

const SCRIPT: &[&str] = &["tsx", "ts", "jsx", "js", "mjs", "mdx", "svelte", "vue"];

fn stem_and_extension(file: &str) -> (&str, &str) {
    match file.rsplit_once('.') {
        Some((stem, extension)) => (stem, extension),
        None => (file, ""),
    }
}

fn join_route(segments: &[&str]) -> String {
    let kept: Vec<&str> = segments
        .iter()
        .copied()
        // Route groups "(marketing)" and parallel slots "@modal" are not in the URL.
        .filter(|segment| {
            !(segment.starts_with('(') && segment.ends_with(')')) && !segment.starts_with('@')
        })
        .filter(|segment| *segment != "index")
        .collect();
    format!("/{}", kept.join("/"))
}

/// The URL a file serves, for file-based routers (Next.js app and pages
/// routers, SvelteKit, Remix, Nuxt).
pub fn route_for(path: &str) -> Option<(RouteKind, String)> {
    let normalized = path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let (file, dirs) = segments.split_last()?;
    let (stem, extension) = stem_and_extension(file);
    if !SCRIPT.contains(&extension) {
        return None;
    }

    // SvelteKit: src/routes/**/+page.svelte and +server.ts
    if let Some(at) = dirs.iter().position(|d| *d == "routes") {
        if dirs.get(at.wrapping_sub(1)) == Some(&"src") {
            let route = join_route(&dirs[at + 1..]);
            return match stem {
                "+page" | "+page.server" => Some((RouteKind::Page, route)),
                "+server" => Some((RouteKind::Api, route)),
                _ => None,
            };
        }
    }
    // Next.js app router: app/**/page.tsx and app/**/route.ts
    if let Some(at) = dirs.iter().position(|d| *d == "app") {
        let route = join_route(&dirs[at + 1..]);
        match stem {
            "page" => return Some((RouteKind::Page, route)),
            "route" => return Some((RouteKind::Api, route)),
            _ => {}
        }
        // Remix: app/routes/a.b.tsx
        if dirs.get(at + 1) == Some(&"routes") && at + 2 == dirs.len() && !stem.starts_with('_') {
            let flat: Vec<&str> = stem.split('.').collect();
            let api = flat.first() == Some(&"api");
            return Some((
                if api { RouteKind::Api } else { RouteKind::Page },
                join_route(&flat),
            ));
        }
    }
    // Nuxt server routes: server/api/**
    if let Some(at) = dirs.iter().position(|d| *d == "server") {
        if dirs.get(at + 1) == Some(&"api") {
            let mut parts = dirs[at + 1..].to_vec();
            parts.push(stem);
            return Some((RouteKind::Api, join_route(&parts)));
        }
    }
    // Next.js pages router and Nuxt pages: pages/**
    if let Some(at) = dirs.iter().position(|d| *d == "pages") {
        if stem.starts_with('_') {
            return None; // _app, _document, _error
        }
        let mut parts = dirs[at + 1..].to_vec();
        parts.push(stem);
        let api = parts.first() == Some(&"api");
        return Some((
            if api { RouteKind::Api } else { RouteKind::Page },
            join_route(&parts),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_app_router_pages_and_api_routes() {
        assert_eq!(
            route_for("app/pricing/page.tsx"),
            Some((RouteKind::Page, "/pricing".into()))
        );
        assert_eq!(
            route_for("src/app/(shop)/checkout/[id]/page.tsx"),
            Some((RouteKind::Page, "/checkout/[id]".into()))
        );
        assert_eq!(
            route_for("app/page.tsx"),
            Some((RouteKind::Page, "/".into()))
        );
        assert_eq!(
            route_for("app/api/stripe/webhook/route.ts"),
            Some((RouteKind::Api, "/api/stripe/webhook".into()))
        );
        assert_eq!(route_for("app/pricing/PricingCard.tsx"), None);
    }

    #[test]
    fn pages_router_sveltekit_remix_and_nuxt() {
        assert_eq!(
            route_for("pages/index.tsx"),
            Some((RouteKind::Page, "/".into()))
        );
        assert_eq!(
            route_for("pages/api/users/[id].ts"),
            Some((RouteKind::Api, "/api/users/[id]".into()))
        );
        assert_eq!(route_for("pages/_app.tsx"), None);
        assert_eq!(
            route_for("src/routes/blog/+page.svelte"),
            Some((RouteKind::Page, "/blog".into()))
        );
        assert_eq!(
            route_for("src/routes/api/posts/+server.ts"),
            Some((RouteKind::Api, "/api/posts".into()))
        );
        assert_eq!(
            route_for("app/routes/settings.billing.tsx"),
            Some((RouteKind::Page, "/settings/billing".into()))
        );
        assert_eq!(
            route_for("server/api/orders.ts"),
            Some((RouteKind::Api, "/api/orders".into()))
        );
        assert_eq!(route_for("src/lib/utils.py"), None);
    }

    #[test]
    fn areas_come_from_paths_and_names() {
        assert_eq!(areas("lib/auth/session.ts", &[]), vec![Area::SignIn]);
        assert_eq!(
            areas("src/billing.rs", &["createCheckoutSession".into()]),
            vec![Area::Payments]
        );
        assert_eq!(areas("lib/session.ts", &[]), vec![Area::SignIn]);
        assert_eq!(
            areas("supabase/migrations/001_init.sql", &[]),
            vec![Area::Database]
        );
        assert_eq!(areas(".env.local", &[]), vec![Area::Settings]);
        assert_eq!(areas("mysite/settings.py", &[]), vec![Area::Settings]);
        assert_eq!(areas("app/settings/page.tsx", &[]), vec![]);
        assert!(areas("components/Button.tsx", &["PricingButton".into()]).is_empty());
    }
}
