pub(crate) mod private {
    use crate::payload::Header;

    pub trait RoleHeader {
        type TxHeader: Header;
        type RxHeader: Header;
    }
}
