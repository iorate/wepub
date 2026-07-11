use uuid::Uuid;

pub(crate) struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Form {
    pub(crate) fn new() -> Self {
        Self {
            // A UUID v4 carries 122 bits of randomness, making a collision
            // with the part contents practically impossible.
            boundary: format!("wepub-{}", Uuid::new_v4()),
            body: Vec::new(),
        }
    }

    pub(crate) fn file(
        mut self,
        name: &str,
        file_name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{}\"; \
                 filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
                self.boundary,
                escape(name),
                escape(file_name),
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub(crate) fn text(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                self.boundary,
                escape(name),
            )
            .as_bytes(),
        );
        self
    }

    pub(crate) fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

// The escaping rule the WHATWG HTML standard specifies for the name and
// filename parameters of multipart/form-data:
// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#multipart-form-data
// Part content types are not covered by the rule and are not escaped.
fn escape(value: &str) -> String {
    value
        .replace('\n', "%0A")
        .replace('\r', "%0D")
        .replace('"', "%22")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary_of(content_type: &str) -> &str {
        content_type
            .strip_prefix("multipart/form-data; boundary=")
            .expect("content type should carry the boundary parameter")
    }

    #[test]
    fn form_serializes_file_and_text_parts_per_rfc_7578() {
        let (content_type, body) = Form::new()
            .file("upload", "addon.zip", "application/zip", b"ZIPDATA")
            .text("channel", "listed")
            .finish();

        let boundary = boundary_of(&content_type);
        let expected = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"upload\"; filename=\"addon.zip\"\r\n\
             Content-Type: application/zip\r\n\
             \r\n\
             ZIPDATA\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"channel\"\r\n\
             \r\n\
             listed\r\n\
             --{boundary}--\r\n"
        );
        assert_eq!(body, expected.as_bytes());
    }

    #[test]
    fn form_serializes_a_single_file_part() {
        let (content_type, body) = Form::new()
            .file("source", "source.zip", "application/zip", b"SRC")
            .finish();

        let boundary = boundary_of(&content_type);
        let expected = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"source\"; filename=\"source.zip\"\r\n\
             Content-Type: application/zip\r\n\
             \r\n\
             SRC\r\n\
             --{boundary}--\r\n"
        );
        assert_eq!(body, expected.as_bytes());
    }

    #[test]
    fn file_parts_keep_binary_data_intact() {
        let data = [0u8, 1, 2, 255, 254, 13, 10, 0];
        let (content_type, body) = Form::new()
            .file("upload", "addon.zip", "application/zip", &data)
            .finish();

        let boundary = boundary_of(&content_type);
        let mut expected = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"upload\"; filename=\"addon.zip\"\r\n\
             Content-Type: application/zip\r\n\
             \r\n"
        )
        .into_bytes();
        expected.extend_from_slice(&data);
        expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        assert_eq!(body, expected);
    }

    #[test]
    fn names_and_file_names_are_escaped_per_the_whatwg_rule() {
        let (_, body) = Form::new()
            .file("na\"me", "file\r\nname.zip", "application/zip", b"X")
            .text("te\rxt", "v")
            .finish();

        let body = String::from_utf8(body).unwrap();
        assert!(
            body.contains("name=\"na%22me\"; filename=\"file%0D%0Aname.zip\""),
            "body: {body}",
        );
        assert!(body.contains("name=\"te%0Dxt\""), "body: {body}");
    }

    #[test]
    fn each_form_generates_a_unique_boundary() {
        let (first, _) = Form::new().finish();
        let (second, _) = Form::new().finish();
        assert_ne!(first, second);
    }
}
