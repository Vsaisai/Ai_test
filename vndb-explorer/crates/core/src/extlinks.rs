//! Build URLs from VNDB `extlinks (site, value)` rows (templates follow VNDB's ExtLinks.pm).

pub fn label(site: &str) -> &str {
    match site {
        "website" => "Official website",
        "steam" => "Steam",
        "gog" => "GOG",
        "itch" | "itch_dev" => "Itch.io",
        "jastusa" => "JAST USA",
        "mg" => "MangaGamer",
        "denpa" => "Denpasoft",
        "fakku" => "Fakku",
        "dlsite" | "dlsiteen" => "DLsite",
        "nutaku" => "Nutaku",
        "johren" => "Johren",
        "kagura" => "Kagura Games",
        "jlist" => "J-List",
        "playasia" => "PlayAsia",
        "googplay" => "Google Play",
        "appstore" => "App Store",
        "nintendo" => "Nintendo",
        "nintendo_jp" => "Nintendo (JP)",
        "nintendo_hk" => "Nintendo (HK)",
        "playstation_na" => "PlayStation Store (NA)",
        "playstation_eu" => "PlayStation Store (EU)",
        "playstation_jp" => "PlayStation Store (JP)",
        "playstation_hk" => "PlayStation Store (HK)",
        "dmm" => "DMM",
        "getchu" => "Getchu",
        "getchudl" => "DL.Getchu",
        "gyutto" => "Gyutto",
        "digiket" => "Digiket",
        "toranoana" => "Toranoana",
        "melon" => "Melonbooks.com",
        "melonjp" => "Melonbooks.co.jp",
        "animateg" => "Animate Games",
        "patreon" => "Patreon",
        "patreonp" => "Patreon post",
        "substar" => "SubscribeStar",
        "booth" | "booth_pub" => "BOOTH",
        "gamejolt" => "Game Jolt",
        "freem" => "Freem!",
        "freegame" => "Freegame Mugen",
        "novelgam" => "NovelGame",
        "renai" => "Renai.us",
        "egs" => "ErogameScape",
        "wikidata" => "Wikidata",
        "wp" => "Wikipedia",
        "boosty" => "Boosty",
        "vk" => "VK",
        "twitter" => "X/Twitter",
        "youtube" => "YouTube",
        "fanbox" => "Fanbox",
        "fantia" => "Fantia",
        "cien" => "Ci-en",
        "kofi" => "Ko-fi",
        "afdian" => "Afdian",
        _ => site,
    }
}

pub fn url(site: &str, v: &str) -> Option<String> {
    let n = || v.parse::<u64>().ok();
    Some(match site {
        "website" => v.to_string(),
        "steam" => format!("https://store.steampowered.com/app/{v}/"),
        "gog" => format!("https://www.gog.com/en/game/{v}"),
        "itch" => {
            let (a, b) = v.split_once('/')?;
            format!("https://{a}.itch.io/{b}")
        }
        "itch_dev" => format!("https://{v}.itch.io/"),
        "jastusa" => format!("https://jastusa.com/games/{v}/vndb"),
        "mg" => format!("https://www.mangagamer.com/r18/detail.php?product_code={v}"),
        "denpa" => format!("https://denpasoft.com/product/{v}/"),
        "fakku" => format!("https://www.fakku.net/games/{v}"),
        "dlsite" => format!("https://www.dlsite.com/home/work/=/product_id/{v}.html"),
        "dlsiteen" => format!("https://www.dlsite.com/eng/work/=/product_id/{v}.html"),
        "nutaku" => format!("https://www.nutaku.net/games/{v}/"),
        "johren" => format!("https://www.johren.games/games/download/{v}/"),
        "kagura" => format!("https://www.kaguragames.com/product/{v}/"),
        "jlist" => format!("https://jlist.com/shop/product/{v}"),
        "playasia" => format!("https://www.play-asia.com/13/70{v}"),
        "googplay" => format!("https://play.google.com/store/apps/details?id={v}"),
        "appstore" => format!("https://apps.apple.com/app/id{v}"),
        "nintendo" => format!("https://www.nintendo.com/store/products/{v}/"),
        "nintendo_jp" => format!("https://store-jp.nintendo.com/item/software/D{v}"),
        "nintendo_hk" => format!("https://store.nintendo.com.hk/{v}"),
        "playstation_na" => format!("https://store.playstation.com/en-us/product/{v}"),
        "playstation_eu" => format!("https://store.playstation.com/en-gb/product/{v}"),
        "playstation_jp" => format!("https://store.playstation.com/ja-jp/product/{v}"),
        "playstation_hk" => format!("https://store.playstation.com/en-hk/product/{v}"),
        "dmm" => format!("https://{v}"),
        "getchu" => format!("http://www.getchu.com/soft.phtml?id={v}"),
        "getchudl" => format!("http://dl.getchu.com/i/item{v}"),
        "gyutto" => format!("https://gyutto.com/i/item{v}"),
        "digiket" => format!("https://www.digiket.com/work/show/_data/ID=ITM{:07}/", n()?),
        "toranoana" => format!("https://ec.toranoana.shop/tora/ec/item/{:012}/", n()?),
        "melon" => format!(
            "https://www.melonbooks.com/index.php?main_page=product_info&products_id=IT{:010}",
            n()?
        ),
        "melonjp" => format!("https://www.melonbooks.co.jp/detail/detail.php?product_id={v}"),
        "animateg" => format!("https://www.animategames.jp/home/detail/{v}"),
        "patreon" => format!("https://www.patreon.com/{v}"),
        "patreonp" => format!("https://www.patreon.com/posts/{v}"),
        "substar" => format!("https://subscribestar.{v}"),
        "booth" => format!("https://booth.pm/en/items/{v}"),
        "booth_pub" => format!("https://{v}.booth.pm/"),
        "gamejolt" => format!("https://gamejolt.com/games/vn/{v}"),
        "freem" => format!("https://www.freem.ne.jp/win/game/{v}"),
        "freegame" => format!("https://freegame-mugen.jp/{v}.html"),
        "novelgam" => format!("https://novelgame.jp/games/show/{v}"),
        "renai" => format!("https://renai.us/game/{v}"),
        "egs" => {
            format!("https://erogamescape.dyndns.org/~ap2/ero/toukei_kaiseki/game.php?game={v}")
        }
        "wikidata" => format!("https://www.wikidata.org/wiki/Q{v}"),
        "wp" => format!("https://en.wikipedia.org/wiki/{v}"),
        "boosty" => format!("https://boosty.to/{v}"),
        "vk" => format!("https://vk.com/{v}"),
        "twitter" => format!("https://x.com/{v}"),
        "youtube" => format!("https://www.youtube.com/@{v}"),
        "fanbox" => format!("https://{v}.fanbox.cc/"),
        "fantia" => format!("https://fantia.jp/fanclubs/{v}"),
        "cien" => format!("https://ci-en.dlsite.com/creator/{v}"),
        "kofi" => format!("https://ko-fi.com/{v}"),
        "afdian" => format!("https://afdian.com/a/{v}"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates() {
        assert_eq!(url("itch", "a/b").unwrap(), "https://a.itch.io/b");
        assert_eq!(
            url("steam", "10").unwrap(),
            "https://store.steampowered.com/app/10/"
        );
        assert_eq!(
            url("digiket", "42").unwrap(),
            "https://www.digiket.com/work/show/_data/ID=ITM0000042/"
        );
        assert!(url("unknown", "x").is_none());
    }
}
