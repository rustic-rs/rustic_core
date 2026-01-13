use {
    cached::proc_macro::cached,
    log::warn,
    nix::unistd::{Gid, Group, Uid, User},
};

const MODE_PERM: u32 = 0o777; // permission bits

// consts from https://pkg.go.dev/io/fs#ModeType
const GO_MODE_DIR: u32 = 0b1000_0000_0000_0000_0000_0000_0000_0000;
const GO_MODE_SYMLINK: u32 = 0b0000_1000_0000_0000_0000_0000_0000_0000;
const GO_MODE_DEVICE: u32 = 0b0000_0100_0000_0000_0000_0000_0000_0000;
const GO_MODE_FIFO: u32 = 0b0000_0010_0000_0000_0000_0000_0000_0000;
const GO_MODE_SOCKET: u32 = 0b0000_0001_0000_0000_0000_0000_0000_0000;
const GO_MODE_SETUID: u32 = 0b0000_0000_1000_0000_0000_0000_0000_0000;
const GO_MODE_SETGID: u32 = 0b0000_0000_0100_0000_0000_0000_0000_0000;
const GO_MODE_CHARDEV: u32 = 0b0000_0000_0010_0000_0000_0000_0000_0000;
const GO_MODE_STICKY: u32 = 0b0000_0000_0001_0000_0000_0000_0000_0000;
const GO_MODE_IRREG: u32 = 0b0000_0000_0000_1000_0000_0000_0000_0000;

// consts from man page inode(7)
const S_IFFORMAT: u32 = 0o170_000; // File mask
const S_IFSOCK: u32 = 0o140_000; // socket
const S_IFLNK: u32 = 0o120_000; // symbolic link
const S_IFREG: u32 = 0o100_000; // regular file
const S_IFBLK: u32 = 0o060_000; // block device
const S_IFDIR: u32 = 0o040_000; // directory
const S_IFCHR: u32 = 0o020_000; // character device
const S_IFIFO: u32 = 0o010_000; // FIFO

const S_ISUID: u32 = 0o4000; // set-user-ID bit (see execve(2))
const S_ISGID: u32 = 0o2000; // set-group-ID bit (see below)
const S_ISVTX: u32 = 0o1000; // sticky bit (see below)

/// map `st_mode` from POSIX (`inode(7)`) to golang's definition (<https://pkg.go.dev/io/fs#ModeType>)
/// Note, that it only sets the bits `os.ModePerm | os.ModeType | os.ModeSetuid | os.ModeSetgid | os.ModeSticky`
/// to stay compatible with the restic implementation
pub const fn map_mode_to_go(mode: u32) -> u32 {
    let mut go_mode = mode & MODE_PERM;

    match mode & S_IFFORMAT {
        S_IFSOCK => go_mode |= GO_MODE_SOCKET,
        S_IFLNK => go_mode |= GO_MODE_SYMLINK,
        S_IFBLK => go_mode |= GO_MODE_DEVICE,
        S_IFDIR => go_mode |= GO_MODE_DIR,
        S_IFCHR => go_mode |= GO_MODE_CHARDEV & GO_MODE_DEVICE, // no idea why go sets both for char devices...
        S_IFIFO => go_mode |= GO_MODE_FIFO,
        // note that POSIX specifies regular files, whereas golang specifies irregular files
        S_IFREG => {}
        _ => go_mode |= GO_MODE_IRREG,
    }

    if mode & S_ISUID > 0 {
        go_mode |= GO_MODE_SETUID;
    }
    if mode & S_ISGID > 0 {
        go_mode |= GO_MODE_SETGID;
    }
    if mode & S_ISVTX > 0 {
        go_mode |= GO_MODE_STICKY;
    }

    go_mode
}

/// map golangs mode definition (<https://pkg.go.dev/io/fs#ModeType>) to `st_mode` from POSIX (`inode(7)`)
/// This is the inverse function to [`map_mode_to_go`]
pub const fn map_mode_from_go(go_mode: u32) -> u32 {
    let mut mode = go_mode & MODE_PERM;

    if go_mode & GO_MODE_SOCKET > 0 {
        mode |= S_IFSOCK;
    } else if go_mode & GO_MODE_SYMLINK > 0 {
        mode |= S_IFLNK;
    } else if go_mode & GO_MODE_DEVICE > 0 && go_mode & GO_MODE_CHARDEV == 0 {
        mode |= S_IFBLK;
    } else if go_mode & GO_MODE_DIR > 0 {
        mode |= S_IFDIR;
    } else if go_mode & (GO_MODE_CHARDEV | GO_MODE_DEVICE) > 0 {
        mode |= S_IFCHR;
    } else if go_mode & GO_MODE_FIFO > 0 {
        mode |= S_IFIFO;
    } else if go_mode & GO_MODE_IRREG > 0 {
        // note that POSIX specifies regular files, whereas golang specifies irregular files
    } else {
        mode |= S_IFREG;
    }

    if go_mode & GO_MODE_SETUID > 0 {
        mode |= S_ISUID;
    }
    if go_mode & GO_MODE_SETGID > 0 {
        mode |= S_ISGID;
    }
    if go_mode & GO_MODE_STICKY > 0 {
        mode |= S_ISVTX;
    }

    mode
}

/// Get the group name for the given gid.
///
/// # Arguments
///
/// * `gid` - The gid to get the group name for.
///
/// # Returns
///
/// The group name for the given gid or `None` if the group could not be found.
#[cached]
pub fn get_group_by_gid(gid: u32) -> Option<String> {
    match Group::from_gid(Gid::from_raw(gid)) {
        Ok(Some(group)) => Some(group.name),
        Ok(None) => None,
        Err(err) => {
            warn!("error getting group from gid {gid}: {err}");
            None
        }
    }
}

/// Get the user name for the given uid.
///
/// # Arguments
///
/// * `uid` - The uid to get the user name for.
///
/// # Returns
///
/// The user name for the given uid or `None` if the user could not be found.
#[cached]
pub fn get_user_by_uid(uid: u32) -> Option<String> {
    match User::from_uid(Uid::from_raw(uid)) {
        Ok(Some(user)) => Some(user.name),
        Ok(None) => None,
        Err(err) => {
            warn!("error getting user from uid {uid}: {err}");
            None
        }
    }
}
