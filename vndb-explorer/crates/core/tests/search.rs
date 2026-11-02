//! End-to-end: build a tiny catalog database, load it and run searches.

use rusqlite::{params, Connection};
use vndb_core::search::{EroMode, LangMode, SearchRequest, Sort};
use vndb_core::{search, Catalog};

type Row<'a> = (u32, &'a str, Vec<(&'a str, u8)>, Vec<(u32, f64)>, u32, bool);

fn build(path: &std::path::Path) {
    let c = Connection::open(path).unwrap();
    vndb_core::db::create_schema(&c).unwrap();
    // tags: Theme(1) > Drama(2) > Tragedy(3); Theme > Comedy(4); Style(5) > ADV(6); Sexual Content(7, ero)
    for (id, name, cat) in [
        (1, "Theme", "cont"),
        (2, "Drama", "cont"),
        (3, "Tragedy", "cont"),
        (4, "Comedy", "cont"),
        (5, "Style", "tech"),
        (6, "ADV", "tech"),
        (7, "Sexual Content", "ero"),
    ] {
        c.execute(
            "INSERT INTO tags VALUES (?1, ?2, 0, 1, 1, ?3, '', '')",
            params![id, cat, name],
        )
        .unwrap();
    }
    for (id, parent) in [(2, 1), (3, 2), (4, 1), (6, 5), (7, 1)] {
        c.execute(
            "INSERT INTO tag_parents VALUES (?1, ?2, 1)",
            params![id, parent],
        )
        .unwrap();
    }
    // vn: id, title, langs, tags, votes, ero
    let vns: Vec<Row> = vec![
        (
            1,
            "Zeta",
            vec![("ja", 0), ("en", 1)],
            vec![(3, 2.0), (4, 1.0)],
            100,
            false,
        ),
        (
            2,
            "Alpha",
            vec![("ja", 0), ("en", 0)],
            vec![(4, 1.0)],
            500,
            false,
        ),
        (
            3,
            "Beta",
            vec![("ja", 0), ("ru", 1)],
            vec![(2, 2.0), (4, 1.0), (6, 2.0)],
            50,
            false,
        ),
        (
            4,
            "Aardvark",
            vec![("en", 0)],
            vec![(2, 1.0), (6, 1.0), (7, 2.5)],
            10,
            true,
        ),
        (
            5,
            "Japanese only",
            vec![("ja", 0)],
            vec![(2, 2.0), (4, 2.0), (6, 2.0)],
            1000,
            false,
        ),
        (
            6,
            "MTL only",
            vec![("ja", 0), ("en", 2)],
            vec![(2, 2.0), (4, 2.0), (6, 2.0)],
            5,
            false,
        ),
    ];
    for (id, title, langs, tags, votes, ero) in vns {
        c.execute(
            "INSERT INTO vn VALUES (?1,'ja',NULL,0,0,?2,7.5,7.5,NULL,0,0,0,'','',20200101,?3,-1,0,'win','')",
            params![id, votes, ero],
        )
        .unwrap();
        c.execute(
            "INSERT INTO vn_titles VALUES (?1,'en',1,?2,NULL)",
            params![id, title],
        )
        .unwrap();
        for (l, k) in langs {
            c.execute("INSERT INTO vn_langs VALUES (?1,?2,?3)", params![id, l, k])
                .unwrap();
        }
        for (t, r) in tags {
            c.execute(
                "INSERT INTO vn_tags VALUES (?1,?2,?3,0,0)",
                params![id, t, r],
            )
            .unwrap();
        }
    }
}

fn catalog() -> (tempdir::Dir, Catalog) {
    let dir = tempdir::Dir::new();
    let p = dir.0.join("c.sqlite");
    build(&p);
    let cat = Catalog::open(&p).unwrap();
    (dir, cat)
}

mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "vndb-core-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn titles(r: &vndb_core::SearchResponse) -> Vec<String> {
    r.items.iter().map(|i| i.title.clone()).collect()
}

#[test]
fn wanted_tags_rank_all_first_then_count_then_alpha() {
    let (_d, cat) = catalog();
    let req = SearchRequest {
        want: vec!["Drama".into(), "Comedy".into(), "ADV".into()],
        ..Default::default()
    };
    let r = search(&cat, &req).unwrap();
    // Drama matches via its sub-tag Tragedy for Zeta. Japanese-only is filtered (readable mode).
    assert_eq!(
        titles(&r),
        vec!["Beta", "MTL only", "Aardvark", "Zeta", "Alpha"]
    );
    assert!(r.items[0].all_wanted);
}

#[test]
fn must_exclude_lang_and_ero() {
    let (_d, cat) = catalog();
    let base = SearchRequest {
        must: vec!["Drama".into()],
        sort: Sort::Title,
        ..Default::default()
    };
    assert_eq!(
        titles(&search(&cat, &base).unwrap()),
        vec!["Aardvark", "Beta", "MTL only", "Zeta"]
    );

    let r = search(
        &cat,
        &SearchRequest {
            exclude: vec!["ADV".into()],
            ..base.clone()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Zeta"]);

    let r = search(
        &cat,
        &SearchRequest {
            ero: EroMode::Hide,
            ..base.clone()
        },
    )
    .unwrap();
    assert!(!titles(&r).contains(&"Aardvark".to_string()));

    let r = search(
        &cat,
        &SearchRequest {
            allow_mtl: false,
            ..base.clone()
        },
    )
    .unwrap();
    assert!(!titles(&r).contains(&"MTL only".to_string()));

    let r = search(
        &cat,
        &SearchRequest {
            lang_mode: LangMode::List,
            langs: vec!["ru".into()],
            ..base.clone()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Beta"]);

    let r = search(
        &cat,
        &SearchRequest {
            lang_mode: LangMode::Any,
            ..base
        },
    )
    .unwrap();
    assert_eq!(r.total, 5);
}

#[test]
fn text_search_and_tag_counts() {
    let (_d, cat) = catalog();
    let r = search(
        &cat,
        &SearchRequest {
            text: Some("aard".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Aardvark"]);
    // Drama count includes VNs tagged only with its child Tragedy
    assert_eq!(cat.tree.get(2).unwrap().vns, 5);
    assert_eq!(
        cat.tree.category(3),
        ("Theme".to_string(), "Drama".to_string())
    );
}

#[test]
fn wanted_tags_do_not_filter_by_default_and_counts_follow_filters() {
    let (_d, cat) = catalog();
    // one wanted tag: VNs without it are still listed, just after the ones that have it
    let r = search(
        &cat,
        &SearchRequest {
            want: vec!["ADV".into()],
            lang_mode: LangMode::Any,
            sort: Sort::Relevance,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.total, 6);
    assert!(r.items[0].all_wanted && !r.items[5].all_wanted);
    // counts are taken over the filtered set (Drama required -> 5 VNs; ADV among them: 4)
    let r = search(
        &cat,
        &SearchRequest {
            must: vec!["Drama".into()],
            lang_mode: LangMode::Any,
            tag_counts: true,
            ..Default::default()
        },
    )
    .unwrap();
    let c = r.tag_counts.unwrap();
    assert_eq!(c.get(&2), Some(&5));
    assert_eq!(c.get(&6), Some(&4));
}
