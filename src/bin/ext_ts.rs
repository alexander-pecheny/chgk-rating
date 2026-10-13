//! A TrueSkill-family rating on places; the rules are in `chgk_rating::rivals::ts`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    chgk_rating::rivals::run(chgk_rating::rivals::System::Ts)
}
