"""Checks DAWproject files against the format's published schemas: Project.xsd and MetaData.xsd
in tests/fixtures/dawproject/, vendored unchanged from github.com/bitwig/dawproject at commit
ee4dcdde75940f30e14e55401a26955a58b8322b (MIT, its LICENSE beside them; licenses/THIRD_PARTY.md):
  Project.xsd   SHA-256 58e2fd9864772850aac3eab1f3de8693857dc5384df6d29fbc320ae1de2347cc
  MetaData.xsd  SHA-256 fb3ba378271770dddbcced8990aba537de3d36ff2d58573523460a699221c99f
The check refuses to run against schema files with other hashes.

The standard library cannot validate XML Schema, so this is a small checker of its own over the
schema files. It reads their global elements, complex types (an extension followed to its base:
the base's content first, then its own; the base's attributes too), attribute declarations
(type, use="required") and simple types (enumerations, lists of them), and holds a document to
them:
  - the root element is the schema's (Project in project.xml, MetaData in metadata.xml);
  - each element's children are what its content model allows, in the order its sequences give,
    each as often as minOccurs and maxOccurs allow, every child then held to its own type;
  - each attribute is declared for the element's type, each required one is there, and its value
    fits its type: xs:int, xs:double, xs:boolean, an enumeration, a list of one, xs:ID (unique in
    the document) or xs:IDREF (naming an xs:ID the document has);
  - text only in elements of a simple type.
And of the package: a zip whose every entry's CRC holds, with project.xml and metadata.xml, and
every File and State path that is not external naming one of its entries.

Run:
  python3 -I dawproject_check.py FILE...   check each .dawproject (or a bare project.xml); print
                                           one line a file, exit 1 when any fault was found
  python3 -I dawproject_check.py --self-test
                                           the checker finds each fault planted in documents of
                                           its own, and passes the good ones; exit 1 if not
"""
import hashlib, io, os, re, sys, zipfile
import xml.etree.ElementTree as ET

XS = "{http://www.w3.org/2001/XMLSchema}"
HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMAS = os.path.join(HERE, "..", "fixtures", "dawproject")
PINNED = {"Project.xsd": "58e2fd9864772850aac3eab1f3de8693857dc5384df6d29fbc320ae1de2347cc",
          "MetaData.xsd": "fb3ba378271770dddbcced8990aba537de3d36ff2d58573523460a699221c99f"}
UNBOUNDED = float("inf")
NCNAME = re.compile(r"^[A-Za-z_][\w.\-]*$")
INT = re.compile(r"^[+-]?\d+$")
DOUBLE = re.compile(r"^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$|^(INF|-INF|NaN)$")


def occurs(node, attribute, default=1):
    value = node.get(attribute)
    if value is None: return default
    return UNBOUNDED if value == "unbounded" else int(value)


class Schema:
    """One schema file: its global elements, complex types and simple types."""

    def __init__(self, path):
        with open(path, "rb") as f: data = f.read()
        digest = hashlib.sha256(data).hexdigest()
        if digest != PINNED[os.path.basename(path)]:
            sys.exit("%s is not the pinned schema (SHA-256 %s)" % (path, digest))
        root = ET.fromstring(data)
        self.elements = {e.get("name"): e for e in root.findall(XS + "element")}
        self.complex = {t.get("name"): t for t in root.findall(XS + "complexType")}
        self.simple = {t.get("name"): t for t in root.findall(XS + "simpleType")}

    # ---- types ----
    def element_type(self, decl):
        """The type of element declaration `decl`: ("complex", node), ("simple", name)."""
        if decl.get("ref") is not None:
            decl = self.elements[decl.get("ref")]
        inline = decl.find(XS + "complexType")
        if inline is not None: return ("complex", inline)
        name = decl.get("type")
        if name in self.complex: return ("complex", self.complex[name])
        return ("simple", name or "xs:string")

    def parts(self, complex_type):
        """The content particles and attribute declarations of `complex_type`, its bases' first."""
        content = complex_type.find(XS + "complexContent")
        if content is not None:
            extension = content.find(XS + "extension")
            base_particles, base_attributes = self.parts(self.complex[extension.get("base")])
            particles, attributes = self.own_parts(extension)
            return base_particles + particles, base_attributes + attributes
        return self.own_parts(complex_type)

    def own_parts(self, node):
        particles = [child for child in node if child.tag in (XS + "sequence", XS + "choice")]
        attributes = node.findall(XS + "attribute")
        return particles, attributes

    def attribute_check(self, attribute):
        """A function telling what is wrong with a value of `attribute`, or None."""
        name = attribute.get("type")
        if name is None:
            inline = attribute.find(XS + "simpleType")
            items = inline.find(XS + "list")
            item = self.value_check(items.get("itemType"))
            return lambda value: next(filter(None, (item(v) for v in value.split())), None)
        return self.value_check(name)

    def value_check(self, name):
        if name == "xs:string": return lambda value: None
        if name == "xs:int": return lambda value: None if INT.match(value) and -2**31 <= int(value) < 2**31 else "not an xs:int"
        if name == "xs:double": return lambda value: None if DOUBLE.match(value) else "not an xs:double"
        if name == "xs:boolean": return lambda value: None if value in ("true", "false", "1", "0") else "not an xs:boolean"
        if name in ("xs:ID", "xs:IDREF"): return lambda value: None if NCNAME.match(value) else "not an NCName"
        restriction = self.simple[name].find(XS + "restriction")
        allowed = {e.get("value") for e in restriction.findall(XS + "enumeration")}
        return lambda value: None if value in allowed else "not one of %s" % sorted(allowed)


class Checker:
    def __init__(self, schema):
        self.schema = schema
        self.faults = []
        self.ids = {}
        self.refs = []
        self.paths = []

    def fault(self, where, what):
        self.faults.append("%s: %s" % (where, what))

    def check(self, root, root_name):
        if root.tag != root_name:
            self.fault(root.tag, "the root element is not %s" % root_name)
            return
        self.element(root, self.schema.element_type(self.schema.elements[root_name]), root_name)
        for where, ref in self.refs:
            if ref not in self.ids: self.fault(where, "IDREF %r names no xs:ID in the document" % ref)

    def element(self, node, kind, where):
        kind, value = kind
        if kind == "simple":
            if len(node): self.fault(where, "a simple element holds elements")
            if node.attrib: self.fault(where, "a simple element holds attributes")
            return
        if (node.text or "").strip() or any((child.tail or "").strip() for child in node):
            self.fault(where, "text in an element of a complex type")
        particles, attributes = self.schema.parts(value)
        self.attributes(node, attributes, where)
        children = list(node)
        names = [child.tag for child in children]
        decls = {}
        for particle in particles: self.declarations(particle, decls)
        for child in children:
            if child.tag not in decls:
                self.fault("%s/%s" % (where, child.tag), "not allowed in %s" % node.tag)
        if all(name in decls for name in names) and len(names) not in self.sequence(particles, names, 0):
            self.fault(where, "children %s do not follow the content model (order, minOccurs, maxOccurs)" % names)
        for child in children:
            if child.tag in decls:
                self.element(child, self.schema.element_type(decls[child.tag]), "%s/%s" % (where, child.tag))

    def declarations(self, particle, decls):
        for child in particle:
            if child.tag == XS + "element":
                decls[child.get("name") or child.get("ref")] = child
            elif child.tag in (XS + "sequence", XS + "choice"):
                self.declarations(child, decls)

    def attributes(self, node, declared, where):
        by_name = {a.get("name"): a for a in declared}
        for name, value in node.attrib.items():
            if name not in by_name:
                self.fault(where, "attribute %r is not declared" % name)
                continue
            wrong = self.schema.attribute_check(by_name[name])(value)
            if wrong: self.fault(where, "%s=%r is %s" % (name, value, wrong))
            kind = by_name[name].get("type")
            if kind == "xs:ID":
                if value in self.ids: self.fault(where, "xs:ID %r given twice" % value)
                self.ids[value] = where
            elif kind == "xs:IDREF":
                self.refs.append((where, value))
        for name, decl in by_name.items():
            if decl.get("use") == "required" and name not in node.attrib:
                self.fault(where, "required attribute %r is missing" % name)
        if node.tag in ("File", "State") and "path" in node.attrib and node.get("external") not in ("true", "1"):
            self.paths.append((where, node.get("path")))

    # ---- content models: the end positions a particle can reach from `start` ----
    def sequence(self, particles, names, start):
        ends = {start}
        for particle in particles:
            ends = set().union(*(self.particle(particle, names, at) for at in ends))
        return ends

    def particle(self, particle, names, start):
        """Ends after minOccurs to maxOccurs rounds of the particle. Past minOccurs a round
        keeps only positions not reached before, so an unbounded particle stops once it finds
        nothing new."""
        low, high = occurs(particle, "minOccurs"), occurs(particle, "maxOccurs")
        reached = {start} if low == 0 else set()
        frontier, count = {start}, 0
        while frontier and count < high:
            count += 1
            frontier = set().union(*(self.once(particle, names, at) for at in frontier))
            if count >= low:
                frontier -= reached
                reached |= frontier
        return reached

    def once(self, particle, names, start):
        tag = particle.tag
        if tag == XS + "element":
            name = particle.get("name") or particle.get("ref")
            return {start + 1} if start < len(names) and names[start] == name else set()
        children = [c for c in particle if c.tag in (XS + "element", XS + "sequence", XS + "choice")]
        if tag == XS + "sequence": return self.sequence(children, names, start)
        if tag == XS + "choice": return set().union(set(), *(self.particle(c, names, start) for c in children))
        return set()


def check_document(schema, text, root_name):
    checker = Checker(schema)
    try:
        root = ET.fromstring(text)
    except ET.ParseError as error:
        return ["not well-formed XML: %s" % error], []
    checker.check(root, root_name)
    return checker.faults, checker.paths


def check_package(data, project, metadata):
    """The faults of the .dawproject file `data`."""
    try:
        archive = zipfile.ZipFile(io.BytesIO(data))
    except zipfile.BadZipFile as error:
        return ["not a zip: %s" % error]
    faults = []
    try:
        broken = archive.testzip()
    except zipfile.BadZipFile as error:
        broken = str(error)
    if broken:
        # The entries are not what was written: reading them further proves nothing.
        return ["entry %s fails its CRC" % broken]
    names = set(archive.namelist())
    for needed in ("project.xml", "metadata.xml"):
        if needed not in names: faults.append("no %s" % needed)
    if "project.xml" in names:
        found, paths = check_document(project, archive.read("project.xml"), "Project")
        faults += ["project.xml " + f for f in found]
        for where, path in paths:
            if path not in names: faults.append("project.xml %s: path %r is not in the package" % (where, path))
    if "metadata.xml" in names:
        found, _ = check_document(metadata, archive.read("metadata.xml"), "MetaData")
        faults += ["metadata.xml " + f for f in found]
    return faults


def schemas():
    return Schema(os.path.join(SCHEMAS, "Project.xsd")), Schema(os.path.join(SCHEMAS, "MetaData.xsd"))


def check_file(path, project, metadata):
    with open(path, "rb") as f: data = f.read()
    if data.startswith(b"PK"): return check_package(data, project, metadata)
    return check_document(project, data, "Project")[0]


# ---------- the self-test: faults planted in documents of the checker's own ----------
GOOD = """<?xml version="1.0" encoding="UTF-8"?>
<Project version="1.0">
  <Application name="t" version="1"/>
  <Transport><Tempo unit="bpm" value="120.0"/><TimeSignature numerator="4" denominator="4"/></Transport>
  <Structure>
    <Track contentType="audio notes" id="t1" name="a">
      <Channel role="regular" destination="m" solo="false" id="c1">
        <Devices><ClapPlugin deviceID="x" deviceName="X" deviceRole="instrument"><State path="plugins/x.bin"/></ClapPlugin></Devices>
        <Mute value="false"/><Pan unit="normalized" value="0.5"/><Volume unit="linear" value="1"/>
      </Channel>
    </Track>
    <Track contentType="audio" id="mt"><Channel role="master" id="m"/></Track>
  </Structure>
  <Arrangement>
    <Lanes timeUnit="beats">
      <Lanes track="t1"><Clips>
        <Clip time="0.0" duration="4" enable="false"><Notes><Note time="0" duration="1e0" channel="0" key="60" vel="1.0"/></Notes></Clip>
        <Clip time="4.0" duration="2.0" playStart="0.2"><Warps contentTimeUnit="seconds" timeUnit="beats">
          <Audio channels="1" duration="0.5" sampleRate="44100" timeUnit="seconds"><File path="audio/a.wav"/></Audio>
          <Warp time="0" contentTime="0"/><Warp time="1" contentTime="0.5"/></Warps></Clip>
      </Clips></Lanes>
    </Lanes>
  </Arrangement>
</Project>
"""
METADATA = '<?xml version="1.0"?>\n<MetaData><Title>t</Title><Artist>a</Artist></MetaData>\n'
PLANTED = [
    ("a missing required attribute", '<Application name="t" version="1"/>', '<Application name="t"/>', "required attribute 'version'"),
    ("an unknown element", "<Transport>", "<Transport><Speed/>", "not allowed in Transport"),
    ("children out of order", '<Mute value="false"/><Pan unit="normalized" value="0.5"/>',
     '<Pan unit="normalized" value="0.5"/><Mute value="false"/>', "do not follow the content model"),
    ("a value outside its enumeration", 'unit="bpm"', 'unit="bananas"', "not one of"),
    ("a list item outside its enumeration", 'contentType="audio notes"', 'contentType="audio drums"', "not one of"),
    ("an IDREF naming nothing", '<Lanes track="t1">', '<Lanes track="nope">', "names no xs:ID"),
    ("a double that is not one", '<Clip time="4.0"', '<Clip time="four"', "not an xs:double"),
    ("an int that is not one", 'key="60"', 'key="sixty"', "not an xs:int"),
    ("a boolean that is not one", 'enable="false"', 'enable="no"', "not an xs:boolean"),
    ("an ID given twice", 'id="c1"', 'id="t1"', "given twice"),
    ("an element more often than maxOccurs", '<Application name="t" version="1"/>',
     '<Application name="t" version="1"/><Application name="u" version="2"/>', "do not follow the content model"),
    ("a required element missing", '<Application name="t" version="1"/>', "", "do not follow the content model"),
    ("an undeclared attribute", '<Track contentType="audio" id="mt">', '<Track contentType="audio" id="mt" hue="red">', "not declared"),
    ("text in a complex element", "<Transport>", "<Transport>words", "text in an element"),
    ("a required child of a Warps missing", '<Warp time="0" contentTime="0"/><Warp time="1" contentTime="0.5"/>', "", "do not follow"),
    ("the wrong root", "<Project version", "<Projekt version", None),
]


def package(project_xml, metadata=METADATA, extra=(("audio/a.wav", b"RIFF"), ("plugins/x.bin", b"\0"))):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("project.xml", project_xml)
        if metadata is not None: archive.writestr("metadata.xml", metadata)
        for name, data in extra: archive.writestr(name, data)
    return buffer.getvalue()


def self_test():
    project, metadata = schemas()
    failures = []
    good = check_package(package(GOOD), project, metadata)
    if good: failures.append("the good package was refused: %s" % good)
    for what, old, new, expect in PLANTED:
        assert GOOD.count(old) >= 1, what
        text = GOOD.replace(old, new, 1)
        if new.startswith("<Projekt"): text = text.replace("</Project>", "</Projekt>")
        faults, _ = check_document(project, text, "Project")
        if not faults or (expect and not any(expect in fault for fault in faults)):
            failures.append("%s was not found (got %s)" % (what, faults))
    cases = [
        ("a path not in the package", package(GOOD, extra=(("plugins/x.bin", b"\0"),)), "is not in the package"),
        ("no metadata.xml", package(GOOD, metadata=None), "no metadata.xml"),
        ("a metadata element out of order", package(GOOD, '<MetaData><Artist>a</Artist><Title>t</Title></MetaData>'), "do not follow"),
    ]
    broken = bytearray(package(GOOD))
    at = broken.index(b"<Project")
    broken[at + 1] ^= 0x20                                   # one byte of project.xml changed
    cases.append(("an entry whose CRC fails", bytes(broken), "fails its CRC"))
    for what, data, expect in cases:
        faults = check_package(data, project, metadata)
        if not any(expect in fault for fault in faults):
            failures.append("%s was not found (got %s)" % (what, faults))
    for failure in failures: print("dawproject_check self-test: " + failure)
    if failures: sys.exit(1)
    print("dawproject_check self-test: the good package passes, %d planted faults found" % (len(PLANTED) + len(cases)))


def main():
    args = sys.argv[1:]
    if args == ["--self-test"]: return self_test()
    if not args: sys.exit(__doc__)
    project, metadata = schemas()
    bad = 0
    for path in args:
        faults = check_file(path, project, metadata)
        if faults:
            bad += 1
            print("%s: %d faults" % (path, len(faults)))
            for fault in faults: print("  " + fault)
        else:
            print("%s: follows the DAWproject 1.0 schema" % path)
    if bad: sys.exit(1)


if __name__ == "__main__":
    main()
