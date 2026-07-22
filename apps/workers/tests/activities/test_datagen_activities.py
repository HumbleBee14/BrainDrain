from src.activities.datagen_activities import dedupe_facets
from src.datagen.protocols import Facet


def test_dedupe_facets_case_insensitive():
    facets = [
        Facet(id="1", label="Billing"),
        Facet(id="2", label="billing"),
        Facet(id="3", label="Refunds"),
    ]
    out = dedupe_facets(facets)
    assert [f.label for f in out] == ["Billing", "Refunds"]
