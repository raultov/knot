use crate::models::{EntityKind, ParsedEntity};

#[allow(dead_code)] // Reserved for future XML parsing
pub(crate) fn extract_entities_xml(
    source: &str,
    file_path: &str,
    repo_name: &str,
) -> Vec<ParsedEntity> {
    let mut entities = Vec::new();

    let doc = match roxmltree::Document::parse(source) {
        Ok(d) => d,
        Err(_) => return entities,
    };

    let root = doc.root();

    // Extract <dependency> elements under <dependencies>
    for deps_node in root
        .descendants()
        .filter(|n| n.tag_name().name() == "dependencies")
    {
        for dep_node in deps_node
            .children()
            .filter(|n| n.tag_name().name() == "dependency")
        {
            if let Some(dep_entity) = extract_dependency(&dep_node, file_path, repo_name) {
                entities.push(dep_entity);
            }
        }
    }

    // Extract <plugin> elements under <build><plugins>
    for build_node in root
        .descendants()
        .filter(|n| n.tag_name().name() == "build")
    {
        for plugins_node in build_node
            .children()
            .filter(|n| n.tag_name().name() == "plugins")
        {
            for plugin_node in plugins_node
                .children()
                .filter(|n| n.tag_name().name() == "plugin")
            {
                if let Some(plugin_entity) = extract_plugin(&plugin_node, file_path, repo_name) {
                    entities.push(plugin_entity);
                }
            }
        }
    }

    entities
}

#[allow(dead_code)] // Reserved for future dependency extraction
fn extract_dependency(
    dep_node: &roxmltree::Node,
    file_path: &str,
    repo_name: &str,
) -> Option<ParsedEntity> {
    let group_id = child_text(dep_node, "groupId")?;
    let artifact_id = child_text(dep_node, "artifactId")?;
    let version = child_text(dep_node, "version").unwrap_or_else(|| "unknown".to_string());
    let scope = child_text(dep_node, "scope");
    let name = format!("{}:{}:{}", group_id, artifact_id, version);

    let signature = scope.map(|s| format!("scope: {}", s));

    Some(ParsedEntity::new(
        &name,
        EntityKind::BuildDependency,
        &name,
        signature,
        Some(format!("Maven dependency: {}", name)),
        "xml",
        file_path,
        1,
        1,
        None,
        repo_name,
    ))
}

#[allow(dead_code)] // Reserved for future plugin extraction
fn extract_plugin(
    plugin_node: &roxmltree::Node,
    file_path: &str,
    repo_name: &str,
) -> Option<ParsedEntity> {
    let group_id = child_text(plugin_node, "groupId")?;
    let artifact_id = child_text(plugin_node, "artifactId")?;
    let version = child_text(plugin_node, "version").unwrap_or_else(|| "unknown".to_string());
    let name = format!("{}:{}:{}", group_id, artifact_id, version);

    Some(ParsedEntity::new(
        &name,
        EntityKind::BuildPlugin,
        &name,
        None,
        Some(format!("Maven plugin: {}", name)),
        "xml",
        file_path,
        1,
        1,
        None,
        repo_name,
    ))
}

#[allow(dead_code)] // Reserved for future child text extraction
fn child_text(parent: &roxmltree::Node, tag: &str) -> Option<String> {
    for child in parent.children() {
        if child.tag_name().name() == tag {
            return child.text().map(|s| s.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_maven_dependency() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>test-app</artifactId>
    <version>1.0.0</version>
    <dependencies>
        <dependency>
            <groupId>org.springframework</groupId>
            <artifactId>spring-core</artifactId>
            <version>5.3.29</version>
        </dependency>
    </dependencies>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "org.springframework:spring-core:5.3.29");
        assert_eq!(entities[0].kind, EntityKind::BuildDependency);
        assert_eq!(entities[0].fqn, "org.springframework:spring-core:5.3.29");
        assert_eq!(entities[0].repo_name, "test-repo");
        assert!(
            entities[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("Maven dependency")
        );
    }

    #[test]
    fn test_extract_multiple_maven_dependencies() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <dependencies>
        <dependency>
            <groupId>com.google.code.gson</groupId>
            <artifactId>gson</artifactId>
            <version>2.10.1</version>
        </dependency>
        <dependency>
            <groupId>org.apache.logging.log4j</groupId>
            <artifactId>log4j-core</artifactId>
            <version>2.20.0</version>
        </dependency>
        <dependency>
            <groupId>junit</groupId>
            <artifactId>junit</artifactId>
            <version>4.13.2</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert_eq!(entities.len(), 3);

        assert_eq!(entities[0].name, "com.google.code.gson:gson:2.10.1");
        assert_eq!(
            entities[1].name,
            "org.apache.logging.log4j:log4j-core:2.20.0"
        );
        assert_eq!(entities[2].name, "junit:junit:4.13.2");

        // Scope should be in the signature
        assert!(entities[2].signature.as_ref().unwrap().contains("test"));
    }

    #[test]
    fn test_extract_maven_plugins() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <build>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>3.11.0</version>
            </plugin>
            <plugin>
                <groupId>org.springframework.boot</groupId>
                <artifactId>spring-boot-maven-plugin</artifactId>
                <version>2.7.14</version>
            </plugin>
        </plugins>
    </build>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert_eq!(entities.len(), 2);

        assert_eq!(
            entities[0].name,
            "org.apache.maven.plugins:maven-compiler-plugin:3.11.0"
        );
        assert_eq!(entities[0].kind, EntityKind::BuildPlugin);
        assert!(
            entities[0]
                .docstring
                .as_ref()
                .unwrap()
                .contains("Maven plugin")
        );

        assert_eq!(
            entities[1].name,
            "org.springframework.boot:spring-boot-maven-plugin:2.7.14"
        );
    }

    #[test]
    fn test_extract_dependencies_and_plugins_together() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <dependencies>
        <dependency>
            <groupId>com.google.code.gson</groupId>
            <artifactId>gson</artifactId>
            <version>2.10.1</version>
        </dependency>
    </dependencies>
    <build>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>3.11.0</version>
            </plugin>
        </plugins>
    </build>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert_eq!(entities.len(), 2);

        let deps: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildDependency)
            .collect();
        assert_eq!(deps.len(), 1);

        let plugins: Vec<_> = entities
            .iter()
            .filter(|e| e.kind == EntityKind::BuildPlugin)
            .collect();
        assert_eq!(plugins.len(), 1);
    }

    #[test]
    fn test_extract_empty_pom() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_extract_dependency_without_version() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <dependencies>
        <dependency>
            <groupId>com.example</groupId>
            <artifactId>managed-lib</artifactId>
        </dependency>
    </dependencies>
</project>"#;

        let entities = extract_entities_xml(source, "pom.xml", "test-repo");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "com.example:managed-lib:unknown");
    }
}
