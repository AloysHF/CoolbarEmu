//! Local consumption receipts for the legacy payment service.

pub(super) fn consumption_receipt(url: &str) -> Option<Vec<u8>> {
    let (authority, path) = url.strip_prefix("http://")?.split_once('/')?;
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    if !(host.eq_ignore_ascii_case("vmt.51coolbar.com") || host == "116.228.32.84")
        || !matches!(port, "80" | "8082")
    {
        return None;
    }
    let (path, query) = path.split_once('?')?;
    if path != "vm/cl" {
        return None;
    }
    let mut fields = std::collections::BTreeMap::new();
    for field in query.split('&') {
        let (key, value) = field.split_once('=')?;
        if fields.insert(key, value).is_some() {
            return None;
        }
    }
    if fields.get("ac") != Some(&"consume")
        || fields.get("pVer") != Some(&"4")
        || fields.get("Subver") != Some(&"2")
    {
        return None;
    }
    let data = fields.get("data")?.as_bytes();
    if data.len() <= 8 || !data[..8].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }

    let (message, _, _) = encoding_rs::GBK.encode("已启用离线游玩。\n按右软键返回游戏。");
    // Accepted request, completed consumption, no nested billing packet,
    // empty metadata, and a byte-length-prefixed GBK receipt.
    let mut response = vec![1, 1, 0, 0, message.len() as u8];
    response.extend_from_slice(&message);
    response.extend_from_slice(&0u32.to_be_bytes()); // No remote wallet balance.
    response.extend_from_slice(&0u32.to_be_bytes()); // No remote ticket balance.
    response.extend_from_slice(&1u16.to_be_bytes()); // One local consumption.
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_contains_complete_bounded_payment_fields() {
        let response = consumption_receipt(
            "http://vmt.51coolbar.com/vm/cl?ac=consume&data=000077D8encoded&pVer=4&Subver=2",
        )
        .unwrap();
        assert_eq!(&response[..4], &[1, 1, 0, 0]);
        let message_end = 5 + response[4] as usize;
        let (message, _, malformed) = encoding_rs::GBK.decode(&response[5..message_end]);
        assert!(!malformed);
        assert!(message.contains("按右软键返回游戏"));
        assert_eq!(response.len(), message_end + 10);
        assert_eq!(&response[message_end..message_end + 8], &[0; 8]);
        assert_eq!(&response[message_end + 8..], &1u16.to_be_bytes());
    }

    #[test]
    fn only_supported_consumption_requests_receive_local_receipts() {
        let query = "vm/cl?ac=consume&data=000077D8encoded&pVer=4&Subver=2";
        for authority in [
            "vmt.51coolbar.com",
            "vmt.51coolbar.com:8082",
            "116.228.32.84:80",
        ] {
            assert!(consumption_receipt(&format!("http://{authority}/{query}")).is_some());
        }
        for url in [
            format!("http://example.com/{query}"),
            format!("http://vmt.51coolbar.com:1234/{query}"),
            format!("http://vmt.51coolbar.com/other?{query}"),
            format!("http://vmt.51coolbar.com/{query}&ac=consume"),
            format!(
                "http://vmt.51coolbar.com/{}",
                query.replace("consume", "lt")
            ),
            format!(
                "http://vmt.51coolbar.com/{}",
                query.replace("pVer=4", "pVer=3")
            ),
            format!(
                "http://vmt.51coolbar.com/{}",
                query.replace("000077D8encoded", "")
            ),
        ] {
            assert!(consumption_receipt(&url).is_none(), "{url}");
        }
    }
}
