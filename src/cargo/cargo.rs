// cargo.rs - Rust package manager

use rustc;
use std;

import rustc::syntax::{ast, codemap, visit};
import rustc::syntax::parse::parser;

import std::fs;
import std::generic_os;
import std::io;
import std::json;
import option;
import option::{none, some};
import result;
import std::map;
import std::os;
import std::run;
import str;
import std::tempfile;
import vec;

tag _src {
    /* Break cycles in package <-> source */
    _source(source);
}

type package = {
//    source: _src,
    name: str,
    uuid: str,
    url: str,
    method: str
};

type source = {
    name: str,
    url: str,
    mutable packages: [package]
};

type cargo = {
    root: str,
    bindir: str,
    libdir: str,
    workdir: str,
    sourcedir: str,
    sources: map::hashmap<str, source>
};

type pkg = {
    name: str,
    vers: str,
    uuid: str,
    desc: option::t<str>,
    sigs: option::t<str>,
    crate_type: option::t<str>
};

fn info(msg: str) {
    io::stdout().write_line(msg);
}

fn warn(msg: str) {
    io::stdout().write_line("warning: " + msg);
}

fn error(msg: str) {
    io::stdout().write_line("error: " + msg);
}

fn load_link(mis: [@ast::meta_item]) -> (option::t<str>,
                                         option::t<str>,
                                         option::t<str>) {
    let name = none;
    let vers = none;
    let uuid = none;
    for a: @ast::meta_item in mis {
        alt a.node {
            ast::meta_name_value(v, {node: ast::lit_str(s), span: _}) {
                alt v {
                    "name" { name = some(s); }
                    "vers" { vers = some(s); }
                    "uuid" { uuid = some(s); }
                    _ { }
                }
            }
        }
    }
    (name, vers, uuid)
}

fn load_pkg(filename: str) -> option::t<pkg> {
    let sess = @{cm: codemap::new_codemap(), mutable next_id: 0};
    let c = parser::parse_crate_from_crate_file(filename, [], sess);

    let name = none;
    let vers = none;
    let uuid = none;
    let desc = none;
    let sigs = none;
    let crate_type = none;

    for a in c.node.attrs {
        alt a.node.value.node {
            ast::meta_name_value(v, {node: ast::lit_str(s), span: _}) {
                alt v {
                    "desc" { desc = some(v); }
                    "sigs" { sigs = some(v); }
                    "crate_type" { crate_type = some(v); }
                    _ { }
                }
            }
            ast::meta_list(v, mis) {
                if v == "link" {
                    let (n, v, u) = load_link(mis);
                    name = n;
                    vers = v;
                    uuid = u;
                }
            }
        }
    }

    alt (name, vers, uuid) {
        (some(name0), some(vers0), some(uuid0)) {
            some({
                name: name0,
                vers: vers0,
                uuid: uuid0,
                desc: desc,
                sigs: sigs,
                crate_type: crate_type})
        }
        _ { ret none; }
    }
}

fn print(s: str) {
    io::stdout().write_line(s);
}

fn rest(s: str, start: uint) -> str {
    if (start >= str::char_len(s)) {
        ""
    } else {
        str::char_slice(s, start, str::char_len(s))
    }
}

fn need_dir(s: str) {
    if fs::path_is_dir(s) { ret; }
    if !fs::make_dir(s, 0x1c0i32) {
        fail #fmt["can't make_dir %s", s];
    }
}

fn parse_source(name: str, j: json::json) -> source {
    alt j {
        json::dict(_j) {
            alt _j.find("url") {
                some(json::string(u)) {
                    ret { name: name, url: u, mutable packages: [] };
                }
                _ { fail "Needed 'url' field in source."; }
            };
        }
        _ { fail "Needed dict value in source."; }
    };
}

fn try_parse_sources(filename: str, sources: map::hashmap<str, source>) {
    if !fs::path_exists(filename)  { ret; }
    let c = io::read_whole_file_str(filename);
    let j = json::from_str(result::get(c));
    alt j {
        some(json::dict(_j)) {
            _j.items { |k, v|
                sources.insert(k, parse_source(k, v));
                log #fmt["source: %s", k];
            }
        }
        _ { fail "malformed sources.json"; }
    }
}

fn load_one_source_package(&src: source, p: map::hashmap<str, json::json>) {
    let name = alt p.find("name") {
        some(json::string(_n)) { _n }
        _ {
            warn("Malformed source json: " + src.name + " (missing name)");
            ret;
        }
    };

    let uuid = alt p.find("uuid") {
        some(json::string(_n)) { _n }
        _ {
            warn("Malformed source json: " + src.name + " (missing uuid)");
            ret;
        }
    };

    let url = alt p.find("url") {
        some(json::string(_n)) { _n }
        _ {
            warn("Malformed source json: " + src.name + " (missing url)");
            ret;
        }
    };

    let method = alt p.find("method") {
        some(json::string(_n)) { _n }
        _ {
            warn("Malformed source json: " + src.name + " (missing method)");
            ret;
        }
    };

    vec::grow(src.packages, 1u, {
        // source: _source(src),
        name: name,
        uuid: uuid,
        url: url,
        method: method
    });
    info("  Loaded package: " + src.name + "/" + name);
}

fn load_source_packages(&c: cargo, &src: source) {
    info("Loading source: " + src.name);
    let dir = fs::connect(c.sourcedir, src.name);
    let pkgfile = fs::connect(dir, "packages.json");
    if !fs::path_exists(pkgfile) { ret; }
    let pkgstr = io::read_whole_file_str(pkgfile);
    let j = json::from_str(result::get(pkgstr));
    alt j {
        some(json::list(js)) {
            for _j: json::json in *js {
                alt _j {
                    json::dict(_p) {
                        load_one_source_package(src, _p);
                    }
                    _ {
                        warn("Malformed source json: " + src.name + " (non-dict pkg)");
                    }
                }
            }
        }
        _ {
            warn("Malformed source json: " + src.name);
        }
    };
}

fn configure() -> cargo {
    let p = alt generic_os::getenv("CARGO_ROOT") {
        some(_p) { _p }
        none. {
            alt generic_os::getenv("HOME") {
                some(_q) { fs::connect(_q, ".cargo") }
                none. { fail "no CARGO_ROOT or HOME"; }
            }
        }
    };

    let sources = map::new_str_hash::<source>();
    try_parse_sources(fs::connect(p, "sources.json"), sources);
    try_parse_sources(fs::connect(p, "local-sources.json"), sources);
    let c = {
        root: p,
        bindir: fs::connect(p, "bin"),
        libdir: fs::connect(p, "lib"),
        workdir: fs::connect(p, "work"),
        sourcedir: fs::connect(p, "sources"),
        sources: sources
    };

    need_dir(c.root);
    need_dir(c.sourcedir);
    need_dir(c.workdir);
    need_dir(c.libdir);
    need_dir(c.bindir);

    sources.keys { |k|
        let s = sources.get(k);
        load_source_packages(c, s);
        sources.insert(k, s);
    };

    c
}

fn for_each_package(c: cargo, b: block(source, package)) {
    c.sources.values({ |v|
        for p in v.packages {
            b(v, p);
        }
    })
}

fn install_one_crate(c: cargo, _path: str, cf: str, _p: pkg) {
    let name = fs::basename(cf);
    let ri = str::index(name, '.' as u8);
    if ri != -1 {
        name = str::slice(name, 0u, ri as uint);
    }
    log #fmt["Installing: %s", name];
    let old = fs::list_dir(".");
    run::run_program("rustc", [name + ".rc"]);
    let new = fs::list_dir(".");
    let created =
        vec::filter::<str>(new, { |n| !vec::member::<str>(n, old) });
    let exec_suffix = os::exec_suffix();
    for ct: str in created {
        if (exec_suffix != "" && str::ends_with(ct, exec_suffix)) ||
            (exec_suffix == "" && !str::starts_with(ct, "lib")) {
            log #fmt["  bin: %s", ct];
            // FIXME: need libstd fs::copy or something
            run::run_program("cp", [ct, c.bindir]);
        } else {
            log #fmt["  lib: %s", ct];
            run::run_program("cp", [ct, c.libdir]);
        }
    }
}

fn install_source(c: cargo, path: str) {
    log #fmt["source: %s", path];
    fs::change_dir(path);
    let contents = fs::list_dir(".");

    log #fmt["contents: %s", str::connect(contents, ", ")];

    let cratefiles =
        vec::filter::<str>(contents, { |n| str::ends_with(n, ".rc") });

    if vec::is_empty(cratefiles) {
        fail "This doesn't look like a rust package (no .rc files).";
    }

    for cf: str in cratefiles {
        let p = load_pkg(cf);
        alt p {
            none. { cont; }
            some(_p) {
                install_one_crate(c, path, cf, _p);
            }
        }
    }
}

fn install_git(c: cargo, wd: str, url: str) {
    run::run_program("git", ["clone", url, wd]);
    install_source(c, wd);
}

fn install_curl(c: cargo, wd: str, url: str) {
    let tarpath = fs::connect(wd, "pkg.tar");
    let p = run::program_output("curl", ["-f", "-s", "-o",
                                         tarpath, url]);
    if p.status != 0 {
        fail #fmt["Fetch of %s failed: %s", url, p.err];
    }
    run::run_program("tar", ["-x", "--strip-components=1",
                             "-C", wd, "-f", tarpath]);
    install_source(c, wd);
}

fn install_file(c: cargo, wd: str, path: str) {
    run::run_program("tar", ["-x", "--strip-components=1",
                             "-C", wd, "-f", path]);
    install_source(c, wd);
}

fn install_resolved(c: cargo, wd: str, key: str) {
    fs::remove_dir(wd);
    let u = "https://rust-package-index.appspot.com/pkg/" + key;
    let p = run::program_output("curl", [u]);
    if p.status != 0 {
        fail #fmt["Fetch of %s failed: %s", u, p.err];
    }
    let j = json::from_str(p.out);
    alt j {
        some (json::dict(_j)) {
            alt _j.find("install") {
                some (json::string(g)) {
                    log #fmt["Resolved: %s -> %s", key, g];
                    cmd_install(c, ["cargo", "install", g]);
                }
                _ { fail #fmt["Bogus install: '%s'", p.out]; }
            }
        }
        _ { fail #fmt["Bad json: '%s'", p.out]; }
    }
}

fn install_package(c: cargo, wd: str, pkg: package) {
    info("Installing with " + pkg.method + " from " + pkg.url + "...");
    if pkg.method == "git" {
        install_git(c, wd, pkg.url);
    } else if pkg.method == "http" {
        install_curl(c, wd, pkg.url);
    } else if pkg.method == "file" {
        install_file(c, wd, pkg.url);
    }
}

fn install_uuid(c: cargo, wd: str, uuid: str) {
    let ps = [];
    for_each_package(c, { |s, p|
        info(#fmt["%s ? %s", p.uuid, uuid]);
        if p.uuid == uuid {
            vec::grow(ps, 1u, (s, p));
        }
    });
    if vec::len(ps) == 1u {
        let (_, p) = ps[0];
        install_package(c, wd, p);
        ret;
    } else if vec::len(ps) == 0u {
        error("No packages.");
        ret;
    }
    error("Found multiple packages:");
    for (s,p) in ps {
        info("  " + s.name + "/" + p.uuid + " (" + p.name + ")");
    }
}

fn install_named(c: cargo, wd: str, name: str) {
    let ps = [];
    for_each_package(c, { |s, p|
        if p.name == name {
            vec::grow(ps, 1u, (s, p));
        }
    });
    if vec::len(ps) == 1u {
        let (_, p) = ps[0];
        install_package(c, wd, p);
        ret;
    } else if vec::len(ps) == 0u {
        error("No packages.");
        ret;
    }
    error("Found multiple packages:");
    for (s,p) in ps {
        info("  " + s.name + "/" + p.uuid + " (" + p.name + ")");
    }
}

fn install_uuid_specific(c: cargo, wd: str, src: str, uuid: str) {
    alt c.sources.find(src) {
        some(s) {
            if vec::any(s.packages, { |p|
                if p.uuid == uuid {
                    install_package(c, wd, p);
                    ret true;
                }
                ret false;
            }) { ret; }
        }
        _ { }
    }
    error("Can't find package " + src + "/" + uuid);
}

fn install_named_specific(c: cargo, wd: str, src: str, name: str) {
    alt c.sources.find(src) {
        some(s) {
            if vec::any(s.packages, { |p|
                if p.name == name {
                    install_package(c, wd, p);
                    ret true;
                }
                ret false;
            }) { ret; }
        }
        _ { }
    }
    error("Can't find package " + src + "/" + name);
}

fn cmd_install(c: cargo, argv: [str]) {
    // cargo install <pkg>
    if vec::len(argv) < 3u {
        cmd_usage();
        ret;
    }

    let wd = alt tempfile::mkdtemp(c.workdir + fs::path_sep(), "") {
        some(_wd) { _wd }
        none. { fail "needed temp dir"; }
    };

    if str::starts_with(argv[2], "uuid:") {
        let uuid = rest(argv[2], 5u);
        let idx = str::index(uuid, '/' as u8);
        if idx != -1 {
            let source = str::slice(uuid, 0u, idx as uint);
            uuid = str::slice(uuid, idx as uint + 1u, str::byte_len(uuid));
            install_uuid_specific(c, wd, source, uuid);
        } else {
            install_uuid(c, wd, uuid);
        }
    } else {
        let name = argv[2];
        let idx = str::index(name, '/' as u8);
        if idx != -1 {
            let source = str::slice(name, 0u, idx as uint);
            name = str::slice(name, idx as uint + 1u, str::byte_len(name));
            install_named_specific(c, wd, source, name);
        } else {
            install_named(c, wd, name);
        }
    }
}

fn sync_one(c: cargo, name: str, src: source) {
    let dir = fs::connect(c.sourcedir, name);
    let pkgfile = fs::connect(dir, "packages.json");
    let url = src.url;
    need_dir(dir);
    info(#fmt["fetching source %s...", name]);
    let p = run::program_output("curl", ["-f", "-s", "-o", pkgfile, url]);
    if p.status != 0 {
        warn(#fmt["fetch for source %s (url %s) failed", name, url]);
    } else {
        info(#fmt["fetched source: %s", name]);
    }
}

fn cmd_sync(c: cargo, argv: [str]) {
    if vec::len(argv) == 3u {
        sync_one(c, argv[2], c.sources.get(argv[2]));
    } else {
        c.sources.items { |k, v|
            sync_one(c, k, v);
        }
    }
}

fn cmd_usage() {
    print("Usage: cargo <verb> [args...]");
    print("  install [source/]package-name        Install by name");
    print("  install uuid:[source/]package-uuid   Install by uuid");
    print("  sync                                 Sync all sources");
    print("  usage                                This");
}

fn main(argv: [str]) {
    if vec::len(argv) < 2u {
        cmd_usage();
        ret;
    }
    let c = configure();
    alt argv[1] {
        "install" { cmd_install(c, argv); }
        "sync" { cmd_sync(c, argv); }
        "usage" { cmd_usage(); }
        _ { cmd_usage(); }
    }
}
