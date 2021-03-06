use crate::common::types as common;
use crate::common::types::{DataSource, VariableName};
use crate::execution::types as execution;
use ordered_float::OrderedFloat;
use std::result;

pub(crate) type PhysicalResult<T> = result::Result<T, PhysicalPlanError>;

#[derive(Fail, PartialEq, Eq, Debug)]
pub enum PhysicalPlanError {
    #[fail(display = "Type Mismatch")]
    #[allow(dead_code)]
    TypeMisMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    DataSource(DataSource, String),
    Filter(Box<Formula>, Box<Node>),
    Map(Vec<Named>, Box<Node>),
    GroupBy(Vec<VariableName>, Vec<NamedAggregate>, Box<Node>),
    Limit(u32, Box<Node>),
    OrderBy(Vec<VariableName>, Vec<Ordering>, Box<Node>),
}

impl Node {
    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(Box<execution::Node>, common::Variables)> {
        match self {
            Node::DataSource(data_source, table_name) => {
                let node = execution::Node::DataSource(data_source.clone(), table_name.clone());
                let variables = common::empty_variables();

                Ok((Box::new(node), variables))
            }
            Node::Filter(formula, source) => {
                let (physical_formula, formula_variables) = formula.physical(physical_plan_creator)?;
                let (child, child_variables) = source.physical(physical_plan_creator)?;

                let return_variables = common::merge(formula_variables, child_variables);
                let filter = execution::Node::Filter(child, physical_formula);
                Ok((Box::new(filter), return_variables))
            }
            Node::Map(expressions, source) => {
                let mut physical_expressions: Vec<execution::Named> = Vec::new();
                let mut total_expression_variables = common::empty_variables();

                for expression in expressions.iter() {
                    let (physical_expression, expression_variables) = expression.physical(physical_plan_creator)?;
                    physical_expressions.push(*physical_expression);
                    total_expression_variables = common::merge(total_expression_variables, expression_variables);
                }

                let (child, child_variables) = source.physical(physical_plan_creator)?;
                let return_variables = common::merge(total_expression_variables, child_variables);

                let node = execution::Node::Map(physical_expressions, child);

                Ok((Box::new(node), return_variables))
            }
            Node::GroupBy(fields, named_aggergates, source) => {
                let mut variables = common::empty_variables();

                let mut physical_aggregates = Vec::new();
                for named_aggregate in named_aggergates.iter() {
                    let (physical_aggregate, aggregate_variables) = named_aggregate.physical(physical_plan_creator)?;
                    variables = common::merge(variables, aggregate_variables);
                    physical_aggregates.push(physical_aggregate);
                }
                let (child, child_variables) = source.physical(physical_plan_creator)?;
                let return_variables = common::merge(variables, child_variables);

                let node = execution::Node::GroupBy(fields.clone(), physical_aggregates, child);

                Ok((Box::new(node), return_variables))
            }
            Node::Limit(row_count, source) => {
                let variables = common::empty_variables();
                let (child, child_variables) = source.physical(physical_plan_creator)?;
                let return_variables = common::merge(variables, child_variables);
                let node = execution::Node::Limit(*row_count, child);
                Ok((Box::new(node), return_variables))
            }
            Node::OrderBy(column_names, orderings, source) => {
                let variables = common::empty_variables();
                let (child, child_variables) = source.physical(physical_plan_creator)?;
                let return_variables = common::merge(variables, child_variables);

                let mut physical_orderings = Vec::new();
                for ordering in orderings.iter() {
                    let physical_ordering = ordering.physical()?;
                    physical_orderings.push(physical_ordering);
                }

                let node = execution::Node::OrderBy(column_names.clone(), physical_orderings, child);
                Ok((Box::new(node), return_variables))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Named {
    Expression(Expression, Option<VariableName>),
    Star,
}

impl Named {
    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(Box<execution::Named>, common::Variables)> {
        match self {
            Named::Expression(expr, name) => {
                let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                Ok((
                    Box::new(execution::Named::Expression(*physical_expr, name.clone())),
                    expr_variables,
                ))
            }
            Named::Star => Ok((Box::new(execution::Named::Star), common::empty_variables())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expression {
    Constant(common::Value),
    Variable(VariableName),
    Logic(Box<Formula>),
    Function(String, Vec<Named>),
}

impl Expression {
    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(Box<execution::Expression>, common::Variables)> {
        match self {
            Expression::Constant(value) => {
                let constant_name = physical_plan_creator.new_constant_name();
                let node = Box::new(execution::Expression::Variable(constant_name.clone()));
                let mut variables = common::Variables::default();
                variables.insert(constant_name, value.clone());

                Ok((node, variables))
            }
            Expression::Variable(name) => {
                let node = Box::new(execution::Expression::Variable(name.clone()));
                let variables = common::empty_variables();

                Ok((node, variables))
            }
            Expression::Logic(formula) => {
                let (expr, variables) = formula.physical(physical_plan_creator)?;
                Ok((Box::new(execution::Expression::Logic(expr)), variables))
            }
            Expression::Function(name, arguments) => {
                let mut physical_args = Vec::new();
                let mut variables = common::empty_variables();

                for arg in arguments.iter() {
                    let (physical_arg, physical_variables) = arg.physical(physical_plan_creator)?;
                    physical_args.push(*physical_arg);
                    variables = common::merge(variables, physical_variables);
                }

                Ok((
                    Box::new(execution::Expression::Function(name.clone(), physical_args)),
                    variables,
                ))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Formula {
    InfixOperator(LogicInfixOp, Box<Formula>, Box<Formula>),
    PrefixOperator(LogicPrefixOp, Box<Formula>),
    Constant(bool),
    Predicate(Relation, Box<Expression>, Box<Expression>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogicInfixOp {
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogicPrefixOp {
    Not,
}

impl Formula {
    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(Box<execution::Formula>, common::Variables)> {
        match self {
            Formula::InfixOperator(op, left_formula, right_formula) => {
                let (left, left_variables) = left_formula.physical(physical_plan_creator)?;
                let (right, right_variables) = right_formula.physical(physical_plan_creator)?;
                let return_variables = common::merge(left_variables, right_variables);

                match op {
                    LogicInfixOp::And => Ok((Box::new(execution::Formula::And(left, right)), return_variables)),
                    LogicInfixOp::Or => Ok((Box::new(execution::Formula::Or(left, right)), return_variables)),
                }
            }
            Formula::PrefixOperator(op, child_formula) => match op {
                LogicPrefixOp::Not => {
                    let (child, child_variables) = child_formula.physical(physical_plan_creator)?;
                    Ok((Box::new(execution::Formula::Not(child)), child_variables))
                }
            },
            Formula::Constant(b) => {
                let node = Box::new(execution::Formula::Constant(*b));
                let variables = common::Variables::default();

                Ok((node, variables))
            }
            Formula::Predicate(relation, left_expr, right_expr) => {
                let (left, left_variables) = left_expr.physical(physical_plan_creator)?;
                let (right, right_variables) = right_expr.physical(physical_plan_creator)?;
                let physical_relation = relation.physical()?;

                let return_variables = common::merge(left_variables, right_variables);
                Ok((
                    Box::new(execution::Formula::Predicate(physical_relation, left, right)),
                    return_variables,
                ))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhysicalPlanCreator {
    counter: u32,
    data_source: DataSource,
}

impl PhysicalPlanCreator {
    pub(crate) fn new(data_source: DataSource) -> Self {
        PhysicalPlanCreator {
            counter: 0,
            data_source,
        }
    }

    pub(crate) fn new_constant_name(&mut self) -> VariableName {
        let constant_name = format!("const_{:09}", self.counter);
        self.counter += 1;
        constant_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NamedAggregate {
    pub(crate) aggregate: Aggregate,
    pub(crate) name_opt: Option<String>,
}

impl NamedAggregate {
    pub(crate) fn new(aggregate: Aggregate, name_opt: Option<String>) -> Self {
        NamedAggregate { aggregate, name_opt }
    }

    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(execution::NamedAggregate, common::Variables)> {
        let (physical_aggregate, expr_variables) = self.aggregate.physical(physical_plan_creator)?;
        Ok((
            execution::NamedAggregate::new(physical_aggregate, self.name_opt.clone()),
            expr_variables,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Aggregate {
    Avg(Named),
    Count(Named),
    First(Named),
    Last(Named),
    Max(Named),
    Min(Named),
    Sum(Named),
    ApproxCountDistinct(Named),
    PercentileDisc(OrderedFloat<f32>, VariableName, Ordering),
    ApproxPercentile(OrderedFloat<f32>, VariableName, Ordering),
}

impl Aggregate {
    pub(crate) fn physical(
        &self,
        physical_plan_creator: &mut PhysicalPlanCreator,
    ) -> PhysicalResult<(execution::Aggregate, common::Variables)> {
        match self {
            Aggregate::Avg(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let avg_aggregate = execution::AvgAggregate::new();
                let aggregate = execution::Aggregate::Avg(avg_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::Count(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let count_aggregate = execution::CountAggregate::new();
                let aggregate = execution::Aggregate::Count(count_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::Sum(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let sum_aggregate = execution::SumAggregate::new();
                let aggregate = execution::Aggregate::Sum(sum_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::First(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let first_aggregate = execution::FirstAggregate::new();
                let aggregate = execution::Aggregate::First(first_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::Last(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let last_aggregate = execution::LastAggregate::new();
                let aggregate = execution::Aggregate::Last(last_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::Min(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let min_aggregate = execution::MinAggregate::new();
                let aggregate = execution::Aggregate::Min(min_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::Max(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let max_aggregate = execution::MaxAggregate::new();
                let aggregate = execution::Aggregate::Max(max_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::ApproxCountDistinct(named) => {
                let mut variables = common::empty_variables();

                let physical_named = match named {
                    Named::Expression(expr, name) => {
                        let (physical_expr, expr_variables) = expr.physical(physical_plan_creator)?;
                        variables = common::merge(variables, expr_variables);
                        execution::Named::Expression(*physical_expr, name.clone())
                    }
                    Named::Star => execution::Named::Star,
                };

                let approx_count_distinct_aggregate = execution::ApproxCountDistinctAggregate::new();
                let aggregate =
                    execution::Aggregate::ApproxCountDistinct(approx_count_distinct_aggregate, physical_named);
                Ok((aggregate, variables))
            }
            Aggregate::PercentileDisc(percentile, column_name, ordering) => {
                let variables = common::empty_variables();
                let physical_ordering = ordering.physical()?;

                let percentile_disc_aggregate = execution::PercentileDiscAggregate::new(*percentile, physical_ordering);
                let aggregate = execution::Aggregate::PercentileDisc(percentile_disc_aggregate, column_name.clone());
                Ok((aggregate, variables))
            }
            Aggregate::ApproxPercentile(percentile, column_name, ordering) => {
                let variables = common::empty_variables();
                let physical_ordering = ordering.physical()?;

                let approx_percentile_aggregate =
                    execution::ApproxPercentileAggregate::new(*percentile, physical_ordering);
                let aggregate =
                    execution::Aggregate::ApproxPercentile(approx_percentile_aggregate, column_name.clone());
                Ok((aggregate, variables))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Relation {
    Equal,
    NotEqual,
    MoreThan,
    LessThan,
    GreaterEqual,
    LessEqual,
}

impl Relation {
    pub(crate) fn physical(&self) -> PhysicalResult<execution::Relation> {
        match self {
            Relation::Equal => Ok(execution::Relation::Equal),
            Relation::NotEqual => Ok(execution::Relation::NotEqual),
            Relation::MoreThan => Ok(execution::Relation::MoreThan),
            Relation::LessThan => Ok(execution::Relation::LessThan),
            Relation::GreaterEqual => Ok(execution::Relation::GreaterEqual),
            Relation::LessEqual => Ok(execution::Relation::LessEqual),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum Ordering {
    Asc,
    Desc,
}

impl Ordering {
    pub(crate) fn physical(&self) -> PhysicalResult<execution::Ordering> {
        match self {
            Ordering::Asc => Ok(execution::Ordering::Asc),
            Ordering::Desc => Ok(execution::Ordering::Desc),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_relation_gen_physical() {
        let rel = Relation::Equal;
        let ans = rel.physical().unwrap();
        let expected = execution::Relation::Equal;

        assert_eq!(expected, ans);
    }

    #[test]
    fn test_formula_gen_physical() {
        let formula = Formula::InfixOperator(
            LogicInfixOp::And,
            Box::new(Formula::Constant(true)),
            Box::new(Formula::Constant(false)),
        );
        let mut physical_plan_creator = PhysicalPlanCreator::new(DataSource::Stdin);
        let (physical_formula, variables) = formula.physical(&mut physical_plan_creator).unwrap();
        let expected_formula = execution::Formula::And(
            Box::new(execution::Formula::Constant(true)),
            Box::new(execution::Formula::Constant(false)),
        );

        let expected_variables = common::Variables::default();

        assert_eq!(expected_formula, *physical_formula);
        assert_eq!(expected_variables, variables);
    }

    #[test]
    fn test_expression_gen_physical() {
        let expr = Expression::Constant(common::Value::Int(1));
        let mut physical_plan_creator = PhysicalPlanCreator::new(DataSource::Stdin);
        let (physical_expr, variables) = expr.physical(&mut physical_plan_creator).unwrap();
        let expected_formula = execution::Expression::Variable("const_000000000".to_string());

        let mut expected_variables = common::Variables::default();
        expected_variables.insert("const_000000000".to_string(), common::Value::Int(1));
        assert_eq!(expected_formula, *physical_expr);
        assert_eq!(expected_variables, variables);
    }

    #[test]
    fn test_filter_with_map_gen_physical() {
        let filtered_formula = Formula::Predicate(
            Relation::Equal,
            Box::new(Expression::Variable("a".to_string())),
            Box::new(Expression::Constant(common::Value::Int(1))),
        );

        let filter = Node::Filter(
            Box::new(filtered_formula),
            Box::new(Node::Map(
                vec![
                    Named::Expression(Expression::Variable("a".to_string()), Some("a".to_string())),
                    Named::Expression(Expression::Variable("b".to_string()), Some("b".to_string())),
                ],
                Box::new(Node::DataSource(DataSource::Stdin, "elb".to_string())),
            )),
        );

        let mut physical_plan_creator = PhysicalPlanCreator::new(DataSource::Stdin);
        let (physical_formula, variables) = filter.physical(&mut physical_plan_creator).unwrap();

        let expected_filtered_formula = execution::Formula::Predicate(
            execution::Relation::Equal,
            Box::new(execution::Expression::Variable("a".to_string())),
            Box::new(execution::Expression::Variable("const_000000000".to_string())),
        );

        let expected_source = execution::Node::Map(
            vec![
                execution::Named::Expression(execution::Expression::Variable("a".to_string()), Some("a".to_string())),
                execution::Named::Expression(execution::Expression::Variable("b".to_string()), Some("b".to_string())),
            ],
            Box::new(execution::Node::DataSource(DataSource::Stdin, "elb".to_string())),
        );

        let expected_filter = execution::Node::Filter(Box::new(expected_source), Box::new(expected_filtered_formula));

        let mut expected_variables = common::Variables::default();
        expected_variables.insert("const_000000000".to_string(), common::Value::Int(1));

        assert_eq!(expected_filter, *physical_formula);
        assert_eq!(expected_variables, variables);
    }

    #[test]
    fn test_group_by_gen_physical() {
        let filtered_formula = Formula::Predicate(
            Relation::Equal,
            Box::new(Expression::Variable("a".to_string())),
            Box::new(Expression::Constant(common::Value::Int(1))),
        );

        let filter = Node::Filter(
            Box::new(filtered_formula),
            Box::new(Node::Map(
                vec![
                    Named::Expression(Expression::Variable("a".to_string()), Some("a".to_string())),
                    Named::Expression(Expression::Variable("b".to_string()), Some("b".to_string())),
                ],
                Box::new(Node::DataSource(DataSource::Stdin, "elb".to_string())),
            )),
        );

        let named_aggregates = vec![
            NamedAggregate::new(
                Aggregate::Avg(Named::Expression(
                    Expression::Variable("a".to_string()),
                    Some("a".to_string()),
                )),
                None,
            ),
            NamedAggregate::new(
                Aggregate::Count(Named::Expression(
                    Expression::Variable("b".to_string()),
                    Some("b".to_string()),
                )),
                None,
            ),
        ];

        let fields = vec!["b".to_string()];
        let group_by = Node::GroupBy(fields, named_aggregates, Box::new(filter));

        let mut physical_plan_creator = PhysicalPlanCreator::new(DataSource::Stdin);
        let (physical_formula, variables) = group_by.physical(&mut physical_plan_creator).unwrap();

        let expected_filtered_formula = execution::Formula::Predicate(
            execution::Relation::Equal,
            Box::new(execution::Expression::Variable("a".to_string())),
            Box::new(execution::Expression::Variable("const_000000000".to_string())),
        );

        let expected_source = execution::Node::Map(
            vec![
                execution::Named::Expression(execution::Expression::Variable("a".to_string()), Some("a".to_string())),
                execution::Named::Expression(execution::Expression::Variable("b".to_string()), Some("b".to_string())),
            ],
            Box::new(execution::Node::DataSource(DataSource::Stdin, "elb".to_string())),
        );

        let expected_filter = execution::Node::Filter(Box::new(expected_source), Box::new(expected_filtered_formula));
        let expected_group_by = execution::Node::GroupBy(
            vec!["b".to_string()],
            vec![
                execution::NamedAggregate::new(
                    execution::Aggregate::Avg(
                        execution::AvgAggregate::new(),
                        execution::Named::Expression(
                            execution::Expression::Variable("a".to_string()),
                            Some("a".to_string()),
                        ),
                    ),
                    None,
                ),
                execution::NamedAggregate::new(
                    execution::Aggregate::Count(
                        execution::CountAggregate::new(),
                        execution::Named::Expression(
                            execution::Expression::Variable("b".to_string()),
                            Some("b".to_string()),
                        ),
                    ),
                    None,
                ),
            ],
            Box::new(expected_filter),
        );

        let mut expected_variables = common::Variables::default();
        expected_variables.insert("const_000000000".to_string(), common::Value::Int(1));

        assert_eq!(expected_group_by, *physical_formula);
        assert_eq!(expected_variables, variables);
    }
}
