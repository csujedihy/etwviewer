use std::ffi::c_void;
use std::sync::{LazyLock, mpsc::Sender};
use std::thread;

use anyhow::{Result, bail};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::core::{PCWSTR, PWSTR};

use crate::store::ParsedEvent;

const PROCESS_TRACE_MODE_EVENT_RECORD: u32 = 0x10000000;

/// Spawns a background thread that opens and processes an ETL file.
pub fn parse_etl(
    path: String,
    sender: Sender<ParsedEvent>,
    done_sender: Sender<()>,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        let result = parse_etl_inner(&path, &sender);
        let _ = done_sender.send(());
        result
    })
}

fn parse_etl_inner(path: &str, sender: &Sender<ParsedEvent>) -> Result<()> {
    let mut path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let sender_ptr = sender as *const Sender<ParsedEvent> as *mut c_void;

    let mut logfile = EVENT_TRACE_LOGFILEW {
        LogFileName: PWSTR(path_wide.as_mut_ptr()),
        Anonymous1: EVENT_TRACE_LOGFILEW_0 {
            ProcessTraceMode: PROCESS_TRACE_MODE_EVENT_RECORD,
        },
        Anonymous2: EVENT_TRACE_LOGFILEW_1 {
            EventRecordCallback: Some(event_record_callback),
        },
        Context: sender_ptr,
        ..Default::default()
    };

    let handle = unsafe { OpenTraceW(&mut logfile) };
    if handle.Value == u64::MAX {
        bail!(
            "OpenTraceW failed for '{}' (last error: {:?})",
            path,
            std::io::Error::last_os_error()
        );
    }

    let status = unsafe { ProcessTrace(&[handle], None, None) };
    unsafe {
        let _ = CloseTrace(handle);
    }

    if status.is_err() {
        bail!("ProcessTrace failed: {:?}", status);
    }

    Ok(())
}

unsafe extern "system" fn event_record_callback(event_record: *mut EVENT_RECORD) {
    let record = unsafe { &*event_record };
    let header = &record.EventHeader;

    let timestamp = header.TimeStamp;
    let cpu = unsafe { record.BufferContext.Anonymous.Anonymous.ProcessorNumber };
    let pid = header.ProcessId;
    let tid = header.ThreadId;
    let event_id = header.EventDescriptor.Id;
    let level = header.EventDescriptor.Level;
    let opcode = header.EventDescriptor.Opcode;

    // Get provider name and formatted message via TDH
    let (provider_name, message, field_ranges) = get_event_info(event_record);

    let parsed = ParsedEvent {
        timestamp,
        cpu,
        pid,
        tid,
        provider_name,
        event_id,
        level,
        opcode,
        message,
        field_ranges,
    };

    let context = record.UserContext;
    if !context.is_null() {
        let sender = unsafe { &*(context as *const Sender<ParsedEvent>) };
        let _ = sender.send(parsed);
    }
}

fn get_event_info(event_record: *mut EVENT_RECORD) -> (String, String, Vec<(usize, usize, u8)>) {
    let mut buffer_size: u32 = 0;

    let status = unsafe {
        TdhGetEventInformation(event_record as *const EVENT_RECORD, None, None, &mut buffer_size)
    };

    if status != 0 && status != 122 {
        let record = unsafe { &*event_record };
        return (
            format!("{{{:?}}}", record.EventHeader.ProviderId),
            String::new(),
            Vec::new(),
        );
    }

    if buffer_size == 0 {
        let record = unsafe { &*event_record };
        return (
            format!("{{{:?}}}", record.EventHeader.ProviderId),
            String::new(),
            Vec::new(),
        );
    }

    let mut buffer = vec![0u8; buffer_size as usize];
    let info_ptr = buffer.as_mut_ptr() as *mut TRACE_EVENT_INFO;

    let status = unsafe {
        TdhGetEventInformation(
            event_record as *const EVENT_RECORD,
            None,
            Some(info_ptr),
            &mut buffer_size,
        )
    };

    if status != 0 {
        let record = unsafe { &*event_record };
        return (
            format!("{{{:?}}}", record.EventHeader.ProviderId),
            String::new(),
            Vec::new(),
        );
    }

    let info = unsafe { &*info_ptr };

    let provider_name = if info.ProviderNameOffset > 0 {
        extract_string_from_buffer(&buffer, info.ProviderNameOffset as usize)
    } else {
        let record = unsafe { &*event_record };
        format!("{{{:?}}}", record.EventHeader.ProviderId)
    };

    let (message, field_ranges) = build_event_message(event_record, info, &buffer);

    (provider_name, message, field_ranges)
}

fn extract_string_from_buffer(buffer: &[u8], offset: usize) -> String {
    if offset >= buffer.len() {
        return String::new();
    }
    let slice = &buffer[offset..];
    let u16_slice: &[u16] =
        unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u16, slice.len() / 2) };
    let len = u16_slice.iter().position(|&c| c == 0).unwrap_or(u16_slice.len());
    String::from_utf16_lossy(&u16_slice[..len])
}

/// Format property values and fill them into the event message template.
/// Falls back to "key=value; ..." if no template is available.
/// Returns (message, field_ranges) where field_ranges marks substituted values.
fn build_event_message(
    event_record: *mut EVENT_RECORD,
    info: &TRACE_EVENT_INFO,
    buffer: &[u8],
) -> (String, Vec<(usize, usize, u8)>) {
    let record = unsafe { &*event_record };
    let prop_count = info.TopLevelPropertyCount as usize;

    if prop_count == 0 {
        return (try_read_raw_string(record), Vec::new());
    }

    // Derive pointer size from event header flags
    let flags = record.EventHeader.Flags as u32;
    let pointer_size: u32 = if flags & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 {
        4
    } else {
        8
    };

    let user_data = if !record.UserData.is_null() && record.UserDataLength > 0 {
        unsafe {
            std::slice::from_raw_parts(
                record.UserData as *const u8,
                record.UserDataLength as usize,
            )
        }
    } else {
        &[]
    };

    let property_array_ptr = info.EventPropertyInfoArray.as_ptr();

    // Track per-property offsets and consumed lengths for PropertyParamLength resolution
    let mut prop_offsets: Vec<usize> = Vec::with_capacity(prop_count);
    let mut prop_consumed: Vec<usize> = Vec::with_capacity(prop_count);
    let mut formatted_values: Vec<String> = Vec::with_capacity(prop_count);
    let mut user_data_offset: usize = 0;

    for i in 0..prop_count {
        let prop = unsafe { &*property_array_ptr.add(i) };
        let prop_flags = prop.Flags.0;

        // Skip struct properties — we don't recurse into sub-properties
        if prop_flags & PropertyStruct.0 != 0 {
            prop_offsets.push(user_data_offset);
            prop_consumed.push(0);
            formatted_values.push(String::new());
            continue;
        }

        let in_type = unsafe { prop.Anonymous1.nonStructType.InType };
        let out_type = unsafe { prop.Anonymous1.nonStructType.OutType };
        let map_name_offset = unsafe { prop.Anonymous1.nonStructType.MapNameOffset };
        let raw_length = unsafe { prop.Anonymous3.length };

        // Resolve property length
        let prop_length: u16 = if prop_flags & PropertyParamLength.0 != 0 {
            // raw_length is an index into the property array whose value holds the real length
            let ref_idx = raw_length as usize;
            if ref_idx < prop_offsets.len() && ref_idx < prop_consumed.len() {
                let ref_off = prop_offsets[ref_idx];
                let ref_len = prop_consumed[ref_idx];
                read_uint_from_user_data(user_data, ref_off, ref_len)
            } else {
                0
            }
        } else {
            raw_length
        };

        // Resolve array count
        let raw_count = unsafe { prop.Anonymous2.count };
        let array_count: u16 = if prop_flags & PropertyParamCount.0 != 0 {
            let ref_idx = raw_count as usize;
            if ref_idx < prop_offsets.len() && ref_idx < prop_consumed.len() {
                let ref_off = prop_offsets[ref_idx];
                let ref_len = prop_consumed[ref_idx];
                read_uint_from_user_data(user_data, ref_off, ref_len)
            } else {
                1
            }
        } else if raw_count > 0 {
            raw_count
        } else {
            1
        };

        prop_offsets.push(user_data_offset);

        if user_data_offset >= user_data.len() {
            prop_consumed.push(0);
            formatted_values.push(String::new());
            continue;
        }

        // Format each array element (usually just 1)
        let mut element_texts: Vec<String> = Vec::new();
        let mut total_consumed: usize = 0;

        // Look up map info once per property, not per array element
        let map_info = get_map_info(event_record, buffer, map_name_offset);
        let map_ptr = map_info
            .as_ref()
            .map(|buf| buf.as_ptr() as *const EVENT_MAP_INFO);

        for _elem in 0..array_count {
            if user_data_offset + total_consumed >= user_data.len() {
                break;
            }
            let prop_data = &user_data[user_data_offset + total_consumed..];

            let value = format_property(
                info,
                map_ptr,
                pointer_size,
                in_type,
                out_type,
                prop_length,
                prop_data,
            );

            match value {
                Some((text, consumed)) => {
                    element_texts.push(text);
                    total_consumed += consumed as usize;
                }
                None => {
                    // TdhFormatProperty failed — skip by known length if possible
                    let skip = if prop_length > 0 {
                        prop_length as usize
                    } else {
                        // For variable-length types with no known length, try common sizes
                        infer_property_size(in_type, pointer_size)
                    };
                    total_consumed += skip;
                    element_texts.push(String::new());
                }
            }
        }

        prop_consumed.push(total_consumed);
        user_data_offset += total_consumed;
        formatted_values.push(element_texts.join(", "));
    }

    // Check for event message template
    if info.EventMessageOffset > 0 {
        let template = extract_string_from_buffer(buffer, info.EventMessageOffset as usize);
        if !template.is_empty() {
            let template = normalize_template(&template);
            return fill_template_from_normalized(&template, &formatted_values);
        }
    }

    // No template — build "key=value; key=value" fallback
    let mut result = String::new();
    let mut ranges: Vec<(usize, usize, u8)> = Vec::new();
    let mut first = true;
    for i in 0..prop_count {
        let prop = unsafe { &*property_array_ptr.add(i) };
        let name = if prop.NameOffset > 0 {
            extract_string_from_buffer(buffer, prop.NameOffset as usize)
        } else {
            format!("prop{}", i)
        };
        if i < formatted_values.len() && !formatted_values[i].is_empty() {
            if !first {
                result.push_str("; ");
            }
            first = false;
            result.push_str(&name);
            result.push('=');
            let val_start = result.len();
            result.push_str(&formatted_values[i]);
            let val_end = result.len();
            ranges.push((val_start, val_end, i as u8 % 8));
        }
    }
    (result, ranges)
}

/// Read a small unsigned integer (1/2/4 bytes) from user_data at a given offset.
/// Used to resolve PropertyParamLength / PropertyParamCount references.
fn read_uint_from_user_data(user_data: &[u8], offset: usize, len: usize) -> u16 {
    if offset + len > user_data.len() {
        return 0;
    }
    let data = &user_data[offset..offset + len];
    match len {
        1 => data[0] as u16,
        2 => u16::from_le_bytes([data[0], data[1]]),
        4 => {
            let v = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            v as u16
        }
        _ => 0,
    }
}

/// Infer the byte size of a property value based on its InType when the schema
/// doesn't provide an explicit length. Returns 0 if unknown (variable-length).
fn infer_property_size(in_type: u16, pointer_size: u32) -> usize {
    // TDH_INTYPE values from tdh.h
    match in_type {
        4 | 13 => 1,         // UINT8, ANSICHAR / BOOLEAN
        5 | 6 => 2,          // INT16, UINT16
        7 | 8 | 14 | 11 => 4, // INT32, UINT32, HEXINT32, FLOAT
        9 | 10 | 15 | 12 | 17 => 8, // INT64, UINT64, HEXINT64, DOUBLE, FILETIME
        16 | 19 => pointer_size as usize, // POINTER, SIZET
        18 => 16,            // GUID
        _ => 0,              // Variable-length or unknown
    }
}

/// Format a single property using TdhFormatProperty.
/// Returns (formatted_string, bytes_consumed) or None on failure.
fn format_property(
    info: &TRACE_EVENT_INFO,
    map_info: Option<*const EVENT_MAP_INFO>,
    pointer_size: u32,
    in_type: u16,
    out_type: u16,
    prop_length: u16,
    prop_data: &[u8],
) -> Option<(String, u16)> {
    // Try with a pre-allocated buffer first to avoid the probe call
    let mut formatted_buf = vec![0u16; 256];
    let mut formatted_size: u32 = formatted_buf.len() as u32;
    let mut consumed: u16 = 0;

    let status = unsafe {
        TdhFormatProperty(
            info as *const TRACE_EVENT_INFO,
            map_info,
            pointer_size,
            in_type,
            out_type,
            prop_length,
            prop_data,
            &mut formatted_size,
            Some(PWSTR(formatted_buf.as_mut_ptr())),
            &mut consumed,
        )
    };

    if status == 0 {
        let len = formatted_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(formatted_size as usize);
        let value = String::from_utf16_lossy(&formatted_buf[..len]);
        return Some((value, consumed));
    }

    if status == 122 && formatted_size > 256 {
        // Buffer too small — retry with the required size
        formatted_buf.resize(formatted_size as usize, 0);
        let status = unsafe {
            TdhFormatProperty(
                info as *const TRACE_EVENT_INFO,
                map_info,
                pointer_size,
                in_type,
                out_type,
                prop_length,
                prop_data,
                &mut formatted_size,
                Some(PWSTR(formatted_buf.as_mut_ptr())),
                &mut consumed,
            )
        };

        if status == 0 {
            let len = formatted_buf
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(formatted_buf.len());
            let value = String::from_utf16_lossy(&formatted_buf[..len]);
            return Some((value, consumed));
        }
    }

    None
}

/// Get event map information for enum-style value lookups.
fn get_map_info(
    event_record: *mut EVENT_RECORD,
    buffer: &[u8],
    map_name_offset: u32,
) -> Option<Vec<u8>> {
    if map_name_offset == 0 {
        return None;
    }

    let map_name = extract_string_from_buffer(buffer, map_name_offset as usize);
    if map_name.is_empty() {
        return None;
    }

    let map_name_wide: Vec<u16> = map_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut map_size: u32 = 0;

    let status = unsafe {
        TdhGetEventMapInformation(
            event_record as *const EVENT_RECORD,
            PCWSTR(map_name_wide.as_ptr()),
            None,
            &mut map_size,
        )
    };

    if status != 122 || map_size == 0 {
        return None;
    }

    let mut map_buf = vec![0u8; map_size as usize];
    let map_ptr = map_buf.as_mut_ptr() as *mut EVENT_MAP_INFO;

    let status = unsafe {
        TdhGetEventMapInformation(
            event_record as *const EVENT_RECORD,
            PCWSTR(map_name_wide.as_ptr()),
            Some(map_ptr),
            &mut map_size,
        )
    };

    if status == 0 {
        Some(map_buf)
    } else {
        None
    }
}

/// Replace %1, %2, ... %N (and %N!format!) placeholders in the template with
/// formatted values. Returns clean text plus byte ranges marking each
/// substituted value for colour highlighting.
static TEMPLATE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"%(\d+)(?:![a-zA-Z0-9]*!)?").unwrap());

fn normalize_template(template: &str) -> String {
    template.replace("\r\n", " ").replace('\n', " ")
}

fn fill_template_from_normalized(
    template: &str,
    values: &[String],
) -> (String, Vec<(usize, usize, u8)>) {
    let re = &*TEMPLATE_RE;

    let mut result = String::new();
    let mut ranges: Vec<(usize, usize, u8)> = Vec::new();
    let mut last_end = 0;

    for caps in re.captures_iter(template) {
        let m = caps.get(0).unwrap();
        // Append template text before this match
        result.push_str(&template[last_end..m.start()]);

        let idx: usize = caps[1].parse().unwrap_or(0);
        if idx >= 1 && idx <= values.len() {
            let i = idx - 1;
            let val_start = result.len();
            result.push_str(&values[i]);
            let val_end = result.len();
            if val_end > val_start {
                ranges.push((val_start, val_end, i as u8 % 8));
            }
        } else {
            // No matching value — keep the placeholder as-is
            result.push_str(m.as_str());
        }

        last_end = m.end();
    }

    // Append trailing template text
    result.push_str(&template[last_end..]);

    let trimmed = result.trim();
    if trimmed.len() < result.len() {
        // Adjust ranges for leading whitespace removal
        let leading = result.len() - result.trim_start().len();
        let new_len = trimmed.len();
        let result = trimmed.to_string();
        let ranges = ranges
            .into_iter()
            .filter_map(|(s, e, c)| {
                let ns = s.saturating_sub(leading);
                let ne = e.saturating_sub(leading).min(new_len);
                if ne > ns { Some((ns, ne, c)) } else { None }
            })
            .collect();
        (result, ranges)
    } else {
        (result, ranges)
    }
}

/// Try to interpret raw UserData as a string when no properties are available.
fn try_read_raw_string(record: &EVENT_RECORD) -> String {
    if record.UserDataLength > 0 && !record.UserData.is_null() {
        let data = unsafe {
            std::slice::from_raw_parts(
                record.UserData as *const u8,
                record.UserDataLength as usize,
            )
        };
        if record.UserDataLength >= 2 {
            let u16_slice: &[u16] = unsafe {
                std::slice::from_raw_parts(data.as_ptr() as *const u16, data.len() / 2)
            };
            let len = u16_slice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(u16_slice.len());
            let s = String::from_utf16_lossy(&u16_slice[..len]);
            if !s.is_empty() && s.chars().all(|c| !c.is_control() || c == '\n' || c == '\r') {
                return s;
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_template_basic() {
        let template = normalize_template("DNS query for %1 type %2 response %3");
        let (msg, ranges) =
            fill_template_from_normalized(&template, &["example.com".into(), "A".into(), "0x0".into()]);
        assert_eq!(msg, "DNS query for example.com type A response 0x0");
        assert_eq!(ranges.len(), 3);
        assert_eq!(&msg[ranges[0].0..ranges[0].1], "example.com");
        assert_eq!(&msg[ranges[1].0..ranges[1].1], "A");
        assert_eq!(&msg[ranges[2].0..ranges[2].1], "0x0");
        assert_eq!(ranges[0].2, 0);
        assert_eq!(ranges[1].2, 1);
        assert_eq!(ranges[2].2, 2);
    }

    #[test]
    fn fill_template_format_specifier() {
        let template = normalize_template("Value is %1!u! and hex %2!x!");
        let (msg, ranges) =
            fill_template_from_normalized(&template, &["42".into(), "FF".into()]);
        assert_eq!(msg, "Value is 42 and hex FF");
        assert_eq!(ranges.len(), 2);
        assert_eq!(&msg[ranges[0].0..ranges[0].1], "42");
        assert_eq!(&msg[ranges[1].0..ranges[1].1], "FF");
    }

    #[test]
    fn fill_template_missing_value() {
        let template = normalize_template("Has %1 and %99");
        let (msg, ranges) = fill_template_from_normalized(&template, &["hello".into()]);
        assert_eq!(msg, "Has hello and %99");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&msg[ranges[0].0..ranges[0].1], "hello");
    }
}
