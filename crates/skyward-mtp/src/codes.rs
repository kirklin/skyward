//! The numbers MTP names its containers, operations, responses, events,
//! object formats, properties and data types by.

/// What a container holds.
pub mod container {
    pub const COMMAND: u16 = 1;
    pub const DATA: u16 = 2;
    pub const RESPONSE: u16 = 3;
    pub const EVENT: u16 = 4;
}

pub mod op {
    pub const GET_DEVICE_INFO: u16 = 0x1001;
    pub const OPEN_SESSION: u16 = 0x1002;
    pub const CLOSE_SESSION: u16 = 0x1003;
    pub const GET_STORAGE_IDS: u16 = 0x1004;
    pub const GET_STORAGE_INFO: u16 = 0x1005;
    pub const GET_NUM_OBJECTS: u16 = 0x1006;
    pub const GET_OBJECT_HANDLES: u16 = 0x1007;
    pub const GET_OBJECT_INFO: u16 = 0x1008;
    pub const GET_OBJECT: u16 = 0x1009;
    pub const GET_THUMB: u16 = 0x100A;
    pub const DELETE_OBJECT: u16 = 0x100B;
    pub const SEND_OBJECT_INFO: u16 = 0x100C;
    pub const SEND_OBJECT: u16 = 0x100D;
    pub const RESET_DEVICE: u16 = 0x1010;
    pub const GET_DEVICE_PROP_DESC: u16 = 0x1014;
    pub const GET_DEVICE_PROP_VALUE: u16 = 0x1015;
    pub const SET_DEVICE_PROP_VALUE: u16 = 0x1016;
    pub const RESET_DEVICE_PROP_VALUE: u16 = 0x1017;
    pub const MOVE_OBJECT: u16 = 0x1019;
    pub const COPY_OBJECT: u16 = 0x101A;
    pub const GET_PARTIAL_OBJECT: u16 = 0x101B;
    pub const GET_OBJECT_PROPS_SUPPORTED: u16 = 0x9801;
    pub const GET_OBJECT_PROP_DESC: u16 = 0x9802;
    pub const GET_OBJECT_PROP_VALUE: u16 = 0x9803;
    pub const SET_OBJECT_PROP_VALUE: u16 = 0x9804;
    pub const GET_OBJECT_PROP_LIST: u16 = 0x9805;
    pub const GET_OBJECT_REFERENCES: u16 = 0x9810;
    pub const SET_OBJECT_REFERENCES: u16 = 0x9811;
    // Android's own operations, which its extension in the device info
    // names: reading past 4 GiB and editing a file in place.
    pub const GET_PARTIAL_OBJECT_64: u16 = 0x95C1;
    pub const SEND_PARTIAL_OBJECT: u16 = 0x95C2;
    pub const TRUNCATE_OBJECT: u16 = 0x95C3;
    pub const BEGIN_EDIT_OBJECT: u16 = 0x95C4;
    pub const END_EDIT_OBJECT: u16 = 0x95C5;
}

pub mod response {
    pub const OK: u16 = 0x2001;
    pub const GENERAL_ERROR: u16 = 0x2002;
    pub const SESSION_NOT_OPEN: u16 = 0x2003;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x2005;
    pub const PARAMETER_NOT_SUPPORTED: u16 = 0x2006;
    pub const INCOMPLETE_TRANSFER: u16 = 0x2007;
    pub const INVALID_STORAGE_ID: u16 = 0x2008;
    pub const INVALID_OBJECT_HANDLE: u16 = 0x2009;
    pub const DEVICE_PROP_NOT_SUPPORTED: u16 = 0x200A;
    pub const STORAGE_FULL: u16 = 0x200C;
    pub const STORE_READ_ONLY: u16 = 0x200E;
    pub const ACCESS_DENIED: u16 = 0x200F;
    pub const NO_THUMBNAIL_PRESENT: u16 = 0x2010;
    pub const PARTIAL_DELETION: u16 = 0x2012;
    pub const STORE_NOT_AVAILABLE: u16 = 0x2013;
    pub const NO_VALID_OBJECT_INFO: u16 = 0x2015;
    pub const INVALID_PARENT_OBJECT: u16 = 0x201A;
    pub const INVALID_DEVICE_PROP_VALUE: u16 = 0x201C;
    pub const INVALID_PARAMETER: u16 = 0x201D;
    pub const SESSION_ALREADY_OPEN: u16 = 0x201E;
    pub const TRANSACTION_CANCELLED: u16 = 0x201F;
    pub const INVALID_OBJECT_PROP_CODE: u16 = 0xA801;
    pub const INVALID_OBJECT_PROP_FORMAT: u16 = 0xA802;
    pub const INVALID_OBJECT_PROP_VALUE: u16 = 0xA803;
    pub const INVALID_DATASET: u16 = 0xA806;
    pub const SPECIFICATION_BY_GROUP_UNSUPPORTED: u16 = 0xA807;
    pub const SPECIFICATION_BY_DEPTH_UNSUPPORTED: u16 = 0xA808;
    pub const OBJECT_PROP_NOT_SUPPORTED: u16 = 0xA80A;
}

pub mod event {
    pub const OBJECT_ADDED: u16 = 0x4002;
    pub const OBJECT_REMOVED: u16 = 0x4003;
    pub const STORE_ADDED: u16 = 0x4004;
    pub const STORE_REMOVED: u16 = 0x4005;
    pub const DEVICE_PROP_CHANGED: u16 = 0x4006;
    pub const OBJECT_INFO_CHANGED: u16 = 0x4007;
}

pub mod format {
    pub const UNDEFINED: u16 = 0x3000;
    /// A folder.
    pub const ASSOCIATION: u16 = 0x3001;
    pub const TEXT: u16 = 0x3004;
    pub const HTML: u16 = 0x3005;
    pub const AIFF: u16 = 0x3007;
    pub const WAV: u16 = 0x3008;
    pub const MP3: u16 = 0x3009;
    pub const AVI: u16 = 0x300A;
    pub const MPEG: u16 = 0x300B;
    pub const ASF: u16 = 0x300C;
    /// An image of no format of its own.
    pub const DEFINED: u16 = 0x3800;
    pub const EXIF_JPEG: u16 = 0x3801;
    pub const TIFF_EP: u16 = 0x3802;
    pub const BMP: u16 = 0x3804;
    pub const GIF: u16 = 0x3807;
    pub const JFIF: u16 = 0x3808;
    pub const PNG: u16 = 0x380B;
    pub const TIFF: u16 = 0x380D;
    pub const JP2: u16 = 0x380F;
    pub const JPX: u16 = 0x3810;
    pub const DNG: u16 = 0x3811;
    pub const HEIF: u16 = 0x3812;
    pub const UNDEFINED_AUDIO: u16 = 0xB900;
    pub const WMA: u16 = 0xB901;
    pub const OGG: u16 = 0xB902;
    pub const AAC: u16 = 0xB903;
    pub const FLAC: u16 = 0xB906;
    pub const UNDEFINED_VIDEO: u16 = 0xB980;
    pub const WMV: u16 = 0xB981;
    pub const MP4_CONTAINER: u16 = 0xB982;
    pub const MP2: u16 = 0xB983;
    pub const THREE_GP_CONTAINER: u16 = 0xB984;
    pub const ABSTRACT_AV_PLAYLIST: u16 = 0xBA05;
    pub const WPL_PLAYLIST: u16 = 0xBA10;
    pub const M3U_PLAYLIST: u16 = 0xBA11;
    pub const PLS_PLAYLIST: u16 = 0xBA14;
    pub const XML_DOCUMENT: u16 = 0xBA82;
    pub const MS_WORD_DOCUMENT: u16 = 0xBA83;
    pub const MS_EXCEL_SPREADSHEET: u16 = 0xBA85;
    pub const MS_POWERPOINT_PRESENTATION: u16 = 0xBA86;
}

/// An object's properties.
pub mod prop {
    pub const STORAGE_ID: u16 = 0xDC01;
    pub const OBJECT_FORMAT: u16 = 0xDC02;
    pub const PROTECTION_STATUS: u16 = 0xDC03;
    pub const OBJECT_SIZE: u16 = 0xDC04;
    pub const OBJECT_FILE_NAME: u16 = 0xDC07;
    pub const DATE_MODIFIED: u16 = 0xDC09;
    pub const PARENT_OBJECT: u16 = 0xDC0B;
    pub const PERSISTENT_UID: u16 = 0xDC41;
    pub const NAME: u16 = 0xDC44;
    pub const DATE_ADDED: u16 = 0xDC4E;
    pub const DISPLAY_NAME: u16 = 0xDCE0;
}

/// The device's own properties.
pub mod device {
    pub const BATTERY_LEVEL: u16 = 0x5001;
    pub const IMAGE_SIZE: u16 = 0x5003;
    pub const SYNCHRONIZATION_PARTNER: u16 = 0xD401;
    pub const DEVICE_FRIENDLY_NAME: u16 = 0xD402;
    pub const SESSION_INITIATOR_VERSION_INFO: u16 = 0xD406;
    pub const PERCEIVED_DEVICE_TYPE: u16 = 0xD407;
}

/// How a value is written.
pub mod kind {
    pub const UINT8: u16 = 0x0002;
    pub const UINT16: u16 = 0x0004;
    pub const UINT32: u16 = 0x0006;
    pub const UINT64: u16 = 0x0008;
    pub const UINT128: u16 = 0x000A;
    pub const STR: u16 = 0xFFFF;
}

/// A property description's form: nothing more, a range, or a date and
/// time.
pub mod form {
    pub const NONE: u8 = 0;
    pub const RANGE: u8 = 1;
    pub const DATE_TIME: u8 = 3;
}

/// The storage the device offers.
pub mod storage {
    pub const FIXED_RAM: u16 = 0x0003;
    pub const HIERARCHICAL: u16 = 0x0002;
    pub const READ_WRITE: u16 = 0x0000;
    pub const READ_ONLY_WITHOUT_DELETE: u16 = 0x0001;
}

/// The class requests the computer sends the MTP interface on the control
/// endpoint.
pub mod request {
    pub const CANCEL: u8 = 0x64;
    pub const RESET: u8 = 0x66;
    pub const GET_DEVICE_STATUS: u8 = 0x67;
}

/// An object's parent, as the computer names the top of a storage.
pub const PARENT_ROOT: u32 = 0xFFFF_FFFF;
