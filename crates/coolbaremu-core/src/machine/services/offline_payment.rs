//! Local consumption receipts for the legacy payment service.

/// Returns a fabricated offline response when the game asks the Coolbar
/// platform for something other than a known payment check.  Games that
/// bootstrap with a `CongfigInfo` or download request should observe the
/// same callback shape as a successful fetch so they continue into play
/// rather than retrying forever on error.
pub(super) fn coolbar_offline_payload(url: &str) -> Option<Vec<u8>> {
    let (authority, path) = url.strip_prefix("http://")?.split_once('/')?;
    let (host, _) = authority.split_once(':').unwrap_or((authority, "80"));
    // Cover every Coolbar-hosted endpoint the firmware might query during
    // boot: download servers, config probes, payment gateways, and the
    // legacy 51coolbar and kvbar subdomains.
    let is_coolbar = host.eq_ignore_ascii_case("res.51coolbar.com")
        || host.eq_ignore_ascii_case("vmt.51coolbar.com")
        || host.eq_ignore_ascii_case("api.51coolbar.com")
        || host.eq_ignore_ascii_case("cdn.51coolbar.com")
        || host.ends_with(".51coolbar.com")
        || host.eq_ignore_ascii_case("116.228.32.84")
        // kvbar.com mirrors the 51coolbar.com fleet for menu and app-down
        // endpoints that older Coolbar titles still reference.
        || host.ends_with(".kvbar.com");
    if !is_coolbar {
        return None;
    }
    // The wireless-menu probe waits for an account answer rather than a
    // download. An empty success leaves the guest parked on its "logging in"
    // screen forever, so keep the offline error that lets it fall back to
    // offline mode (issue #186).
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if host.eq_ignore_ascii_case("wmix.kvbar.com") && path == "wmix/d" {
        return None;
    }
    // The boot loader fetches CongfigInfo to learn the level table before it
    // releases the loading screen. With an empty answer it retries forever.
    if let Some(rest) = url.split_once("/CongfigInfo") {
        if rest.0.contains("/download/") {
            return Some(checksummed_body(
                b"#1(0~5)#2(6~15:1)#3(16~24)#4(25~34)#5(35~44)",
            ));
        }
    }
    // Starting a stage first asks the payment gateway to log the player in.
    // An empty body leaves the title waiting on its data-update page.
    if let Some(response) = login_receipt(url) {
        return Some(response);
    }
    // Zero-length payload: a successful download that carried no body.
    Some(Vec::new())
}

/// Append the checksum the firmware's downloader expects.
///
/// A downloaded body ends with a four-byte little-endian additive checksum:
/// the sum of the payload's big-endian 32-bit words plus the big-endian
/// remainder bytes. The in-guest downloader recomputes that value over
/// everything but the last four bytes and compares it, so an offline
/// stand-in has to generate the trailer instead of reusing a cached file.
fn checksummed_body(payload: &[u8]) -> Vec<u8> {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 4 <= payload.len() {
        sum = sum.wrapping_add(u32::from_be_bytes([
            payload[index],
            payload[index + 1],
            payload[index + 2],
            payload[index + 3],
        ]));
        index += 4;
    }
    let mut tail = 0u32;
    for byte in &payload[index..] {
        tail = (tail << 8).wrapping_add(*byte as u32);
    }
    let mut body = payload.to_vec();
    body.extend_from_slice(&sum.wrapping_add(tail).to_le_bytes());
    body
}

pub(super) fn consumption_receipt(url: &str) -> Option<Vec<u8>> {
    cl_receipt(url, "consume", "已启用离线游玩。\n按右软键返回游戏。")
}

/// Offline answer for the payment gateway's login probe (`ac=lt`).
///
/// The login action shares the receipt envelope the consumption path uses,
/// so the same accepted/completed/message/balance layout satisfies it. Its
/// reply is what releases the title's data-update page.
fn login_receipt(url: &str) -> Option<Vec<u8>> {
    cl_receipt(url, "lt", "已启用离线游玩。")
}

/// Build the receipt envelope for a `vm/cl` action.
fn cl_receipt(url: &str, action: &str, text: &str) -> Option<Vec<u8>> {
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
    if fields.get("ac") != Some(&action)
        || fields.get("pVer") != Some(&"4")
        || fields.get("Subver") != Some(&"2")
    {
        return None;
    }
    let data = fields.get("data")?.as_bytes();
    if data.len() <= 8 || !data[..8].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }

    let (message, _, _) = encoding_rs::GBK.encode(text);
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
    fn login_request_receives_receipt_instead_of_empty_body() {
        let url = "http://vmt.51coolbar.com/vm/cl?ac=lt&data=000077D8encoded&pVer=4&Subver=2";
        let body = coolbar_offline_payload(url).unwrap();
        assert_eq!(&body[..4], &[1, 1, 0, 0]);
        assert_eq!(body.len(), 5 + body[4] as usize + 10);
        // Unrelated platform endpoints keep the zero-length success.
        assert!(
            coolbar_offline_payload("http://namr.51coolbar.com/appdown/d")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn wireless_login_probe_keeps_offline_error() {
        // A successful empty answer leaves the guest waiting for account data
        // on its "logging in" screen; the offline error is what releases it.
        assert!(
            coolbar_offline_payload(
                "http://wmix.kvbar.com/wmix/d?mver=0&kver=46&sc=5&platform=5&type=a&nver=1&mode=&imei=&qsms=20&scaddr=&appid=1002"
            )
            .is_none()
        );
        // The download and menu probes on the same fleet still complete.
        for url in [
            "http://namr.51coolbar.com/appdown/d?mver=0&kver=10&type=a",
            "http://sd.kvbar.com/dmenu/d?mode=9935&ver=1",
            "http://dl.kvbar.com/appdown/d?mver=1&kver=10",
        ] {
            assert!(coolbar_offline_payload(url).unwrap().is_empty(), "{url}");
        }
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
