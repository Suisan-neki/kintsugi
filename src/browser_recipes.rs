//! 画面で選ぶ同梱ブラウザ試験。選択・閲覧だけでは取得も実行も行わない。
use serde_json::{Value, json};
use std::io;

pub fn catalog() -> io::Result<Value> {
    let normal: Value =
        serde_json::from_str(include_str!("../examples/openemr-browser/soap-recipe.json"))
            .map_err(io::Error::other)?;
    let mut stopped = normal.clone();
    stopped["trial"]["fault"] = json!("database_stopped");
    let actions = stopped["browser"]["actions"]
        .as_array_mut()
        .ok_or_else(|| io::Error::other("SOAP試験の操作がありません"))?;
    let save = actions
        .iter()
        .position(|a| a["id"] == "save-soap")
        .ok_or_else(|| io::Error::other("SOAPの保存操作がありません"))?;
    let preparation: Vec<_> = actions.drain(..save).collect();
    stopped["browser"]["preparation"]
        .as_array_mut()
        .ok_or_else(|| io::Error::other("SOAP試験の準備がありません"))?
        .extend(preparation);
    stopped["trial"]["checks"]
        .as_array_mut()
        .ok_or_else(|| io::Error::other("SOAP試験の条件がありません"))?
        .retain(|c| c["id"].as_str().is_some_and(|id| id.starts_with("soap-")));
    let mut recipes = vec![];
    for (id, title, recipe) in [
        ("openemr-soap", "OpenEMR：SOAPの保存と再表示", normal),
        (
            "openemr-soap-db-stopped",
            "OpenEMR：SOAPの保存直前にDBを停止",
            stopped,
        ),
    ] {
        let parsed: crate::execution::Recipe =
            serde_json::from_value(recipe.clone()).map_err(io::Error::other)?;
        crate::execution::validate_recipe(&parsed)?;
        recipes.push(json!({"id":id,"title":title,"recipe":recipe,
            "origin":"examples/openemr-browser/soap-recipe.json",
            "note":"架空の患者と診療記録で試します。Gitの参照版と、実行するイメージのコードが同一であることは未確認です。"}));
    }
    Ok(json!({"schema_version":1,"recipes":recipes}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_database_prepares_soap_but_still_attempts_save() {
        let value = catalog().unwrap();
        let normal = &value["recipes"][0]["recipe"];
        let stopped = &value["recipes"][1]["recipe"];
        assert_eq!(normal["trial"]["fault"], "normal");
        assert_eq!(stopped["trial"]["fault"], "database_stopped");
        assert_eq!(stopped["browser"]["actions"][0]["id"], "save-soap");
        assert!(
            stopped["browser"]["preparation"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["id"] != "save-soap")
        );
        assert_eq!(normal["source"], stopped["source"]);
        assert_eq!(normal["containers"], stopped["containers"]);
        assert!(
            stopped["trial"]["checks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["id"].as_str().unwrap().starts_with("soap-"))
        );
    }
}
