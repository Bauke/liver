use anyhow::Result;
use liver::watch;

#[test]
#[ignore = "Only run this manually, this test doesn't exit by itself."]
fn test_watch() -> Result<()> {
  watch("tests/")
}
