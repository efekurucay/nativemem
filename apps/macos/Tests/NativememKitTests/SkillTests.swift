import Foundation
import Testing
@testable import NativememKit

@Suite struct SkillTests {
    private func entries() throws -> [Item] {
        try decodeContract(ItemsDocument.self, from: fixture("skill_items")).items
    }

    @Test func decodesSkillItems() throws {
        let items = try entries()
        #expect(items.map(\.kind) == ["skill", "skill", "instruction"])
        #expect(items[0].isSkill && !items[2].isSkill)
        #expect(items[0].nativeId == "skill:demo-skill")
        #expect(items[0].body.isEmpty)
        #expect(items[1].replicaOf == ItemRef(origin: "claude", nativeId: "skill:demo-skill"))
    }

    @Test func skillsFoldCopiesAndStayOutOfGlobalAndProjects() throws {
        let index = MemoryIndex(entries: try entries())
        #expect(index.skills.count == 1)
        let skill = try #require(index.skills.first)
        #expect(skill.livesIn.map(\.agent) == ["claude", "pi"])
        #expect(skill.livesIn.map(\.label) == ["original", "copy"])
        #expect(index.items(in: .skills).map(\.primary.name) == ["demo-skill"])
        #expect(index.items(in: .global).map(\.primary.name) == ["style"])
        #expect(index.projects.map(\.count) == [1])
        #expect(index.items(in: .folder(path: "/")).isEmpty)
    }

    @Test func projectSkillsLiveInTheFolderTreeAndShowEachCopyPath() throws {
        var global = try #require(entries().first)
        var project = global
        project.nativeId = "skill:app-123/demo-skill"
        project.scope = Scope(type: "project", path: "/work/app")
        project.path = "/work/app/.claude/skills/demo-skill"
        var copy = project
        copy.agent = "pi"
        copy.path = "/work/app/.pi/skills/demo-skill"
        copy.replicaOf = ItemRef(origin: "claude", nativeId: project.nativeId)
        var other = project
        other.nativeId = "skill:other-456/demo-skill"
        other.scope = Scope(type: "project", path: "/work/other")
        other.path = "/work/other/.claude/skills/demo-skill"
        global.path = "/home/.claude/skills/demo-skill"
        let index = MemoryIndex(entries: [global, project, copy, other])
        #expect(index.skills.count == 1)
        #expect(index.items(in: .skills).map(\.primary.path) == [global.path])
        #expect(index.items(in: .global).isEmpty)
        #expect(index.projects.count == 2)
        #expect(index.items(in: .folder(path: "/work")).count == 2)
        let selected = try #require(index.items(in: .folderOnly(path: "/work/app")).first)
        #expect(selected.primary.path == project.path)
        #expect(selected.copies.map(\.path) == [copy.path])
        #expect(selected.project.path == "/work/app")
        #expect(index.sections(for: .folder(path: "/work")).count == 2)
    }

    @Test func skillSearchMatchesNameAndDescription() throws {
        let index = MemoryIndex(entries: try entries())
        #expect(index.items(in: .skills, matching: "Does demo").count == 1)
        #expect(index.items(in: .skills, matching: "nothing").isEmpty)
    }

    @Test func decodesTheSkillText() throws {
        let shown = try decodeContract(SkillText.self, from: fixture("skill_show"))
        #expect(shown.agent == "claude")
        #expect(shown.name == "demo-skill")
        #expect(shown.text.hasPrefix("---\nname: demo-skill"))
    }

    @Test func statusCountsSkillsApartFromEntries() throws {
        let status = try decodeContract(StatusDocument.self, from: fixture("status_skills"))
        #expect(status.agents.map(\.skills) == [2, 2])
        #expect(status.logicalItems == 1)
        #expect(status.agentsInSync == 2)
        #expect(status.agents[0].skipped.map(\.label) == ["demo-skill"])
        let earlier = try decodeContract(StatusDocument.self, from: fixture("status"))
        #expect(earlier.agents.map(\.skills) == [0, 0])
    }

    @Test func pausedSkillsReadAsWords() throws {
        let json = """
        { "agents": [ { "id": "claude", "installed": true, "status": "action_required", "paused": ["skill"],
                        "checks": [] } ] }
        """
        let document = try decodeContract(DoctorDocument.self, from: Data(json.utf8))
        #expect(document.agents[0].pausedText == "skills")
    }
}
