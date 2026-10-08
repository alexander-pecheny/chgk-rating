//! rating.chgk.gg reimplemented; the rules are in `chgk_rating::rivals::gg`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    chgk_rating::rivals::run(chgk_rating::rivals::System::Gg)
}
