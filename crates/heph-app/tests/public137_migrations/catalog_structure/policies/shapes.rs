//! Closed PG17 analyzed-policy nodes. Every listed field remains in comparison.
use super::super::super::catalog_constraints::expression::FUNCTION_FIELDS;
use std::collections::BTreeSet;

pub(super) fn fields(tag: &str, kind: Option<&str>) -> BTreeSet<String> {
    if tag == "FUNCEXPR" {
        return FUNCTION_FIELDS
            .iter()
            .map(|field| (*field).into())
            .collect();
    }
    let schema = match tag {
        "CONST" => {
            "consttype consttypmod constcollid constlen constbyval constisnull location constvalue"
        }
        "VAR" => {
            "varno varattno vartype vartypmod varcollid varnullingrels varlevelsup varnosyn varattnosyn location"
        }
        "PARAM" => "paramkind paramid paramtype paramtypmod paramcollid location",
        "BOOLEXPR" => "boolop args location",
        "OPEXPR" | "DISTINCTEXPR" | "NULLIFEXPR" => {
            "opno opfuncid opresulttype opretset opcollid inputcollid args location"
        }
        "SCALARARRAYOPEXPR" => "opno opfuncid hashfuncid negfuncid useOr inputcollid args location",
        "NULLTEST" => "arg nulltesttype argisrow location",
        "BOOLEANTEST" => "arg booltesttype location",
        "COERCEVIAIO" => "arg resulttype resultcollid coerceformat location",
        "RELABELTYPE" => "arg resulttype resulttypmod resultcollid relabelformat location",
        "COALESCEEXPR" => "coalescetype coalescecollid args location",
        "ARRAYEXPR" => "array_typeid array_collid element_typeid elements multidims location",
        "CASEEXPR" => "casetype casecollid arg args defresult location",
        "CASEWHEN" => "expr result location",
        "CASETESTEXPR" => "typeId typeMod collation",
        "ROWEXPR" => "args row_typeid row_format colnames location",
        "ROWCOMPAREEXPR" => "rctype opnos opfamilies inputcollids largs rargs",
        "SQLVALUEFUNCTION" => "op type typmod location",
        "SUBLINK" => "subLinkType subLinkId testexpr operName subselect location",
        "QUERY" => {
            "commandType querySource canSetTag utilityStmt resultRelation hasAggs hasWindowFuncs hasTargetSRFs hasSubLinks hasDistinctOn hasRecursive hasModifyingCTE hasForUpdate hasRowSecurity isReturn cteList rtable rteperminfos jointree mergeActionList mergeTargetRelation mergeJoinCondition targetList override onConflict returningList groupClause groupDistinct groupingSets havingQual windowClause distinctClause sortClause limitOffset limitCount limitOption rowMarks setOperations constraintDeps withCheckOptions stmt_location stmt_len"
        }
        "RANGETBLENTRY" => match kind.expect("RTE kind") {
            "0" => {
                "alias eref rtekind relid inh relkind rellockmode perminfoindex tablesample lateral inFromCl securityQuals"
            }
            "2" => {
                "alias eref rtekind jointype joinmergedcols joinaliasvars joinleftcols joinrightcols join_using_alias lateral inFromCl securityQuals"
            }
            _ => panic!("unsupported policy RTE kind"),
        },
        "RTEPERMISSIONINFO" => {
            "relid inh requiredPerms checkAsUser selectedCols insertedCols updatedCols"
        }
        "RANGETBLREF" => "rtindex",
        "FROMEXPR" => "fromlist quals",
        "JOINEXPR" => {
            "jointype isNatural larg rarg usingClause join_using_alias quals alias rtindex"
        }
        "TARGETENTRY" => "expr resno resname ressortgroupref resorigtbl resorigcol resjunk",
        "ALIAS" => "aliasname colnames",
        "STRING" => "sval",
        _ => panic!("unsupported policy AST node {tag}"),
    };
    schema
        .split_whitespace()
        .map(|field| format!(":{field}"))
        .collect()
}
