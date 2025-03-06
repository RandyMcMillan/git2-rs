use ::time::at;
use ::time::Timespec;
use clap::Parser;
use git2::{Commit, DiffOptions, ObjectType, Repository, Signature, Time};
use git2::{DiffFormat, Error, Pathspec};
use nostr::prelude::*;
use nostr::*;
use std::str;

use sha2::{Digest, Sha256};

#[derive(Parser)]
struct Args {
    #[structopt(name = "topo-order", long)]
    /// sort commits in topological order
    flag_topo_order: bool,
    #[structopt(name = "date-order", long)]
    /// sort commits in date order
    flag_date_order: bool,
    #[structopt(name = "reverse", long)]
    /// sort commits in reverse
    flag_reverse: bool,
    #[structopt(name = "author", long)]
    /// author to sort by
    flag_author: Option<String>,
    #[structopt(name = "committer", long)]
    /// committer to sort by
    flag_committer: Option<String>,
    #[structopt(name = "pat", long = "grep")]
    /// pattern to filter commit messages by
    flag_grep: Option<String>,
    #[structopt(name = "dir", long = "git-dir")]
    /// alternative git directory to use
    flag_git_dir: Option<String>,
    #[structopt(name = "skip", long)]
    /// number of commits to skip
    flag_skip: Option<usize>,
    #[structopt(name = "max-count", short = 'n', long)]
    /// maximum number of commits to show
    flag_max_count: Option<usize>,
    #[structopt(name = "merges", long)]
    /// only show merge commits
    flag_merges: bool,
    #[structopt(name = "no-merges", long)]
    /// don't show merge commits
    flag_no_merges: bool,
    #[structopt(name = "no-min-parents", long)]
    /// don't require a minimum number of parents
    flag_no_min_parents: bool,
    #[structopt(name = "no-max-parents", long)]
    /// don't require a maximum number of parents
    flag_no_max_parents: bool,
    #[structopt(name = "max-parents")]
    /// specify a maximum number of parents for a commit
    flag_max_parents: Option<usize>,
    #[structopt(name = "min-parents")]
    /// specify a minimum number of parents for a commit
    flag_min_parents: Option<usize>,
    #[structopt(name = "patch", long, short, default_value = "false")]
    /// show commit diff
    flag_patch: bool,
    #[structopt(name = "hashlist", long, default_value = "false")]
    /// show commit hash list
    flag_hashlist: bool,
    #[structopt(name = "commit")]
    arg_commit: Vec<String>,
    #[structopt(name = "spec", last = true)]
    arg_spec: Vec<String>,
}

fn run(args: &Args) -> Result<(), Error> {
    let path = args.flag_git_dir.as_ref().map(|s| &s[..]).unwrap_or(".");
    let repo = Repository::open(path)?;
    let mut revwalk = repo.revwalk()?;

    // Prepare the revwalk based on CLI parameters
    let base = if args.flag_reverse {
        git2::Sort::REVERSE
    } else {
        git2::Sort::NONE
    };
    revwalk.set_sorting(
        base | if args.flag_topo_order {
            git2::Sort::TOPOLOGICAL
        } else if args.flag_date_order {
            git2::Sort::TIME
        } else {
            git2::Sort::NONE
        },
    )?;
    for commit in &args.arg_commit {
        if commit.starts_with('^') {
            let obj = repo.revparse_single(&commit[1..])?;
            revwalk.hide(obj.id())?;
            continue;
        }
        let revspec = repo.revparse(commit)?;
        if revspec.mode().contains(git2::RevparseMode::SINGLE) {
            revwalk.push(revspec.from().unwrap().id())?;
        } else {
            let from = revspec.from().unwrap().id();
            let to = revspec.to().unwrap().id();
            revwalk.push(to)?;
            if revspec.mode().contains(git2::RevparseMode::MERGE_BASE) {
                let base = repo.merge_base(from, to)?;
                let o = repo.find_object(base, Some(ObjectType::Commit))?;
                revwalk.push(o.id())?;
            }
            revwalk.hide(from)?;
        }
    }
    if args.arg_commit.is_empty() {
        revwalk.push_head()?;
    }

    // Prepare our diff options and pathspec matcher
    let (mut diffopts, mut diffopts2) = (DiffOptions::new(), DiffOptions::new());
    for spec in &args.arg_spec {
        diffopts.pathspec(spec);
        diffopts2.pathspec(spec);
    }
    let ps = Pathspec::new(args.arg_spec.iter())?;

    // Filter our revwalk based on the CLI parameters
    macro_rules! filter_try {
        ($e:expr) => {
            match $e {
                Ok(t) => t,
                Err(e) => return Some(Err(e)),
            }
        };
    }
    let revwalk = revwalk
        .filter_map(|id| {
            let id = filter_try!(id);
            let commit = filter_try!(repo.find_commit(id));
            let parents = commit.parents().len();
            if parents < args.min_parents() {
                return None;
            }
            if let Some(n) = args.max_parents() {
                if parents >= n {
                    return None;
                }
            }
            if !args.arg_spec.is_empty() {
                match commit.parents().len() {
                    0 => {
                        let tree = filter_try!(commit.tree());
                        let flags = git2::PathspecFlags::NO_MATCH_ERROR;
                        if ps.match_tree(&tree, flags).is_err() {
                            return None;
                        }
                    }
                    _ => {
                        let m = commit.parents().all(|parent| {
                            match_with_parent(&repo, &commit, &parent, &mut diffopts)
                                .unwrap_or(false)
                        });
                        if !m {
                            return None;
                        }
                    }
                }
            }
            if !sig_matches(&commit.author(), &args.flag_author) {
                return None;
            }
            if !sig_matches(&commit.committer(), &args.flag_committer) {
                return None;
            }
            if !log_message_matches(commit.message(), &args.flag_grep) {
                return None;
            }
            Some(Ok(commit))
        })
        .skip(args.flag_skip.unwrap_or(0))
        .take(args.flag_max_count.unwrap_or(!0));

    // print!
    {}
    for commit in revwalk {
        let commit = commit?;
        let key_from_commit = generate(&commit);
        //println!("\nkey_from_commit:secret_key:{}\n", key_from_commit.secret_key().expect("").to_secret_hex());
        //println!("key_from_commit:{:?}", key_from_commit);
        //print_hashlist(&commit);

        let metadata = Metadata::new()
            .name("username")
            .display_name("My Username")
            .about("Description")
            .picture(Url::parse("https://example.com/avatar.png").expect(""))
            .banner(Url::parse("https://example.com/banner.png").expect(""))
            .nip05("username@example.com")
            .lud16("yuki@getalby.com")
            .custom_field("custom_field", "my value");

        let event: Event = EventBuilder::metadata(&metadata)
            .to_event(&key_from_commit)
            .expect("");
        // Convert client nessage to JSON
        let json = ClientMessage::event(event).as_json();
        println!("{json}");

        // New text note
        let event: Event = EventBuilder::text_note("Hello from rust-nostr", [])
            .to_event(&key_from_commit)
            .expect("");

        //// New POW text note
        let event: Event = EventBuilder::text_note("My first POW text n", [])
            .to_pow_event(&key_from_commit, 10)
            .expect("");

        // Convert client nessage to JSON
        let json = ClientMessage::event(event).as_json();
        println!("{json}");

        //--hashlist true present in cli args
        if args.flag_hashlist {
            print_hashlist(&commit);
            continue;
        }

        print_commit(&commit); //print the commit header

        //no -p arg false
        if !args.flag_patch || commit.parents().len() > 1 {
            continue;
        }

        //a function
        //-p true present in cli args
        //parse and print diff body

        let a = if commit.parents().len() == 1 {
            let parent = commit.parent(0)?;
            Some(parent.tree()?)
        } else {
            None
        };
        let b = commit.tree()?;
        let diff = repo.diff_tree_to_tree(a.as_ref(), Some(&b), Some(&mut diffopts2))?;
        //
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            match line.origin() {
                ' ' => print!("{}", line.origin()),
                '+' => print!("{}", line.origin()),
                '-' => print!("{}", line.origin()),
                _ => {}
            }

            let mut hasher = Sha256::new();
            //prime hasher with key_from_commit
            hasher.update(format!("{:?}", key_from_commit.secret_key()));
            //the line.content is stacked on top of key_from_commit
            hasher.update(line.content());
            let result = hasher.finalize();
            let hash = format!("{:x}", result);
            print!("{}({})\n", str::from_utf8(line.content()).unwrap(), hash);
            true
        })?;
        //a function end
    }

    Ok(())
}

fn sig_matches(sig: &Signature, arg: &Option<String>) -> bool {
    match *arg {
        Some(ref s) => {
            sig.name().map(|n| n.contains(s)).unwrap_or(false)
                || sig.email().map(|n| n.contains(s)).unwrap_or(false)
        }
        None => true,
    }
}

fn log_message_matches(msg: Option<&str>, grep: &Option<String>) -> bool {
    match (grep, msg) {
        (&None, _) => true,
        (&Some(_), None) => false,
        (&Some(ref s), Some(msg)) => msg.contains(s),
    }
}

fn print_hashlist(commit: &Commit) -> String {
    let key_from_commit = &format!("{}", format!("{:0>64}", commit.id()));
    let keys = Keys::parse(key_from_commit).expect("");

    let secret_key = keys.secret_key().expect("");
    let public_key = keys.public_key();

    print!(
        "Secret key (bech32): {:}\n",
        secret_key.to_bech32().unwrap()
    );
    print!(
        "Public key (bech32): {:}\n",
        public_key.to_bech32().unwrap()
    );

    print!("{:0>64}\n", format!("{:0>64}", commit.id()));
    if commit.parents().len() > 1 {
        //println!(">>");
        for id in commit.parent_ids() {
            //print!(" {:.8}", id);
            print!("parent:{:0>64}\n", id);
        }
        println!();
    }
    String::from("")
}

fn print_commit(commit: &Commit) {
    //format and print commit header and diff
    if commit.parents().len() > 1 {
        print!("Merge:");
        for id in commit.parent_ids() {
            print!(" {:.8}", id);
        }
        println!();
    }
    println!("commit {}", commit.id());

    let author = commit.author();
    println!("Author: {}", author);
    print_time(&author.when(), "Date:   ");
    println!();

    for line in String::from_utf8_lossy(commit.message_bytes()).lines() {
        println!("    {}", line);
    }
    println!();
}

fn print_time(time: &Time, prefix: &str) {
    let (offset, sign) = match time.offset_minutes() {
        n if n < 0 => (-n, '-'),
        n => (n, '+'),
    };
    let (hours, minutes) = (offset / 60, offset % 60);
    let ts = Timespec::new(time.seconds() + (time.offset_minutes() as i64) * 60, 0);
    let time = at(ts);

    println!(
        "{}{} {}{:02}{:02}",
        prefix,
        time.strftime("%a %b %e %T %Y").unwrap(),
        sign,
        hours,
        minutes
    );
}

fn match_with_parent(
    repo: &Repository,
    commit: &Commit,
    parent: &Commit,
    opts: &mut DiffOptions,
) -> Result<bool, Error> {
    let a = parent.tree()?;
    let b = commit.tree()?;
    let diff = repo.diff_tree_to_tree(Some(&a), Some(&b), Some(opts))?;
    Ok(diff.deltas().len() > 0)
}

impl Args {
    fn min_parents(&self) -> usize {
        if self.flag_no_min_parents {
            return 0;
        }
        self.flag_min_parents
            .unwrap_or(if self.flag_merges { 2 } else { 0 })
    }

    fn max_parents(&self) -> Option<usize> {
        if self.flag_no_max_parents {
            return None;
        }
        self.flag_max_parents
            .or(if self.flag_no_merges { Some(1) } else { None })
    }
}

pub fn generate(commit: &Commit) -> Keys {
    //println!("{}",commit.id());
    let key_from_commit = &format!("{}", format!("{:0>64}", commit.id()));
    //let keys = Keys::generate();
    let keys = Keys::parse(key_from_commit).expect("");
    //println!("\nsecret-key:{}\n", keys.secret_key().expect("").to_secret_hex());

    //let public_key = keys.public_key();
    //let secret_key = keys.secret_key().expect("");

    //print!("Public key (hex): {}\n", public_key);

    //print!("•{}\n", public_key);
    //print!("Public key (bech32): {}\n", public_key.to_bech32()?);
    //print!("{}\n", public_key.to_bech32()?);
    //
    //print!("Secret key (hex): {}\n", keys.secret_key().expect("").to_secret_hex());
    //
    //print!("•{}\n", keys.secret_key()?.to_secret_hex());
    //print!("Secret key (bech32): {:?}\n", secret_key.to_bech32());
    //print!("{}\n", secret_key.to_bech32()?);

    // Parse keys directly from secret key
    //let keys = Keys::parse("secret-key").expect("");
    //println!("\nsecret-key:{}\n", keys.secret_key()?.to_secret_hex());

    // Parse secret key and construct keys
    //let secret_key = SecretKey::parse("6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e").expect("");
    //let keys = Keys::new(secret_key);

    //print!("•{}\n", keys.secret_key()?.to_secret_hex());
    // Restore from bech32
    //let secret_key = SecretKey::from_bech32("nsec1j4c6269y9w0q2er2xjw8sv2ehyrtfxq3jwgdlxj6qfn8z4gjsq5qfvfk99").expect("");
    //let keys = Keys::new(secret_key);
    //print!("•{}\n", keys.secret_key()?.to_secret_hex());

    // Restore from hex
    //let secret_key = SecretKey::from_hex("6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e").expect("");
    //let keys = Keys::new(secret_key);
    //print!("•{}\n", keys.secret_key()?.to_secret_hex());

    keys
}

fn main() -> Result<()> {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => {}
        Err(e) => println!("error: {}", e),
    }

    // Generate new random keys
    let my_keys = Keys::generate();

    // Or use your already existing (from hex or bech32)
    let my_keys = Keys::parse("hex-or-bech32-secret-key")?;

    // Show bech32 public key
    let bech32_pubkey: String = my_keys.public_key().to_bech32()?;
    println!("Bech32 PubKey: {}", bech32_pubkey);

    let metadata = Metadata::new()
        .name("username")
        .display_name("My Username")
        .about("Description")
        .picture(Url::parse("https://example.com/avatar.png")?)
        .banner(Url::parse("https://example.com/banner.png")?)
        .nip05("username@example.com")
        .lud16("yuki@getalby.com")
        .custom_field("custom_field", "my value");

    let event: Event = EventBuilder::metadata(&metadata).to_event(&my_keys)?;

    // New text note
    let event: Event = EventBuilder::text_note("Hello from rust-nostr", []).to_event(&my_keys)?;

    // New POW text note
    let event: Event = EventBuilder::text_note("My first POW text note from rust-nostr", [])
        .to_pow_event(&my_keys, 20)?;

    // Convert client nessage to JSON
    let json = ClientMessage::event(event).as_json();
    println!("{json}");

    Ok(())
}
