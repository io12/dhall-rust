use std::fs::File;
use std::io::{Read,Write,BufReader,BufRead};
use std::collections::HashMap;

use abnf_to_pest::{PestRuleSettings, abnf_to_pest};

fn main() -> std::io::Result<()> {
    // TODO: upstream changes to grammar
    // let abnf_path = "../dhall-lang/standard/dhall.abnf";
    let abnf_path = "src/dhall.abnf";
    let visibility_path = "src/dhall.pest.visibility";
    let pest_path = "src/dhall.pest";
    println!("cargo:rerun-if-changed={}", abnf_path);
    println!("cargo:rerun-if-changed={}", visibility_path);

    let mut file = File::open(abnf_path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    data.push('\n' as u8);

    let mut rule_settings: HashMap<String, PestRuleSettings> = HashMap::new();
    for line in BufReader::new(File::open(visibility_path)?).lines() {
        let line = line?;
        if line.len() >= 2 && &line[0..2] == "# " {
            rule_settings.insert(line[2..].into(), PestRuleSettings { visible: false, ..Default::default() });
        } else {
            rule_settings.insert(line, PestRuleSettings { visible: true, ..Default::default() });
        }
    }
    rule_settings.insert("simple_label".to_owned(), PestRuleSettings {
        visible: true,
        replace: Some("
              keyword_raw ~ simple_label_next_char+
            | !keyword_raw ~ simple_label_first_char ~ simple_label_next_char*
        ".to_owned()),
    });

    let mut file = File::create(pest_path)?;
    writeln!(&mut file, "// AUTO-GENERATED FILE. See build.rs.")?;
    writeln!(&mut file, "{}", abnf_to_pest(&data, &rule_settings)?)?;
    writeln!(&mut file, "keyword_raw = _{{ let_raw | in_raw | if_raw | then_raw | else_raw }}")?;
    writeln!(&mut file, "final_expression = {{ SOI ~ complete_expression ~ EOI }}")?;

    Ok(())
}
