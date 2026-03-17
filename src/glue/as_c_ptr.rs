use std::ffi::{c_char, CString};

pub trait IntoRawCPtr {
    type Output;

    fn into_raw_c_ptr(self) -> *mut Self::Output;
}

impl IntoRawCPtr for String {
    type Output = c_char;

    fn into_raw_c_ptr(self) -> *mut Self::Output {
        CString::new(self).unwrap().into_raw()
    }
}
