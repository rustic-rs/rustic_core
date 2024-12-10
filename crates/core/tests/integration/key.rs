use std::collections::HashMap;

use rustic_core::{
    repofile::{KeyFile, KeyId},
    KeyOptions,
};

use super::{set_up_repo, RepoOpen};
use anyhow::Result;
use rstest::rstest;

#[rstest]
fn test_key_commands(set_up_repo: Result<RepoOpen>) -> Result<()> {
    let repo = set_up_repo?;
    let key_id = repo.key_id();

    // we should have just a single key now
    let keys: Vec<KeyId> = repo.list()?.collect();
    assert_eq!(&keys, &[*key_id]);

    // add key
    let opts = KeyOptions::default()
        .hostname("my_host".to_string())
        .username("my_user".to_string())
        .with_created(true);
    let key_id2 = repo.add_key("my_pass", &opts)?;
    assert_ne!(key_id, &key_id2);

    // check if we have the correct 2 keys
    let keys: HashMap<_, KeyFile> = repo.stream_files()?.filter_map(Result::ok).collect();
    assert_eq!(keys.len(), 2);
    assert!(keys.contains_key(key_id));
    let keyfile2 = keys.get(&key_id2).unwrap();
    assert_eq!(keyfile2.hostname, Some("my_host".to_string()));
    assert_eq!(keyfile2.username, Some("my_user".to_string()));
    assert!(keyfile2.created.is_some());

    // try to remove the used repository key - which should fail
    assert!(repo.delete_key(&key_id.to_string()).is_err());

    // try to remove the added key
    repo.delete_key(&key_id2.to_string())?;

    // we should have just a single key now
    let keys: Vec<KeyId> = repo.list()?.collect();
    assert_eq!(&keys, &[*key_id]);

    Ok(())
}
