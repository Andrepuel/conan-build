use std::mem::MaybeUninit;
use zeromq_sys_sample::zmq_version;

fn main() {
    let mut major = MaybeUninit::uninit();
    let mut minor = MaybeUninit::uninit();
    let mut patch = MaybeUninit::uninit();
    let version = unsafe {
        zmq_version(major.as_mut_ptr(), minor.as_mut_ptr(), patch.as_mut_ptr());
        (
            major.assume_init(),
            minor.assume_init(),
            patch.assume_init(),
        )
    };

    println!("ZMQ version {version:?}");
}
