//! A small edit of the recognition model's ONNX graph: the best class of
//! every output step is taken on the device.
//!
//! The recognition model ends in a softmax over every character class
//! (about 18,400 for PP-OCRv5) at every output step. Copying those
//! probabilities back to the host costs more than running the model: a
//! batch of 320 steps is about 23 MB, and greedy CTC decoding only needs
//! the best class and its probability per step. [`with_best_class`]
//! appends an `ArgMax` and a `ReduceMax` over the class axis to the graph
//! and makes their results the graph's outputs, so a run returns
//! `batch × steps` indices and probabilities instead.
//!
//! ONNX models are protocol buffers (`onnx.proto`, proto2). The edit only
//! needs to walk the top-level fields of the model and of its graph, so it
//! reads and writes the wire format directly instead of pulling in a
//! protobuf code generator: every field it does not touch is copied byte
//! for byte.

use crate::error::OcrError;

/// The name of the appended output with the best class per step (`int64`,
/// `batch × steps`).
pub(crate) const BEST_CLASS: &str = "blurrd_best_class";
/// The name of the appended output with its probability (`float`,
/// `batch × steps`).
pub(crate) const BEST_PROB: &str = "blurrd_best_prob";

/// `ModelProto.graph`.
const MODEL_GRAPH: u32 = 7;
/// `GraphProto.node`.
const GRAPH_NODE: u32 = 1;
/// `GraphProto.output`.
const GRAPH_OUTPUT: u32 = 12;
/// `ValueInfoProto.name`.
const VALUE_INFO_NAME: u32 = 1;
/// `ValueInfoProto.type`.
const VALUE_INFO_TYPE: u32 = 2;
/// `TypeProto.tensor_type`.
const TYPE_TENSOR: u32 = 1;
/// `TypeProto.Tensor.elem_type`.
const TENSOR_ELEM_TYPE: u32 = 1;
/// `NodeProto` fields.
const NODE_INPUT: u32 = 1;
const NODE_OUTPUT: u32 = 2;
const NODE_NAME: u32 = 3;
const NODE_OP_TYPE: u32 = 4;
const NODE_ATTRIBUTE: u32 = 5;
/// `AttributeProto` fields.
const ATTR_NAME: u32 = 1;
const ATTR_I: u32 = 3;
const ATTR_INTS: u32 = 8;
const ATTR_TYPE: u32 = 20;
/// `AttributeProto.AttributeType` values.
const ATTR_TYPE_INT: u64 = 2;
const ATTR_TYPE_INTS: u64 = 7;
/// `TensorProto.DataType` values.
const ELEM_FLOAT: u64 = 1;
const ELEM_INT64: u64 = 7;

/// Protocol buffer wire types.
const WIRE_VARINT: u8 = 0;
const WIRE_FIXED64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_FIXED32: u8 = 5;

/// The recognition `model` with the best class of every step and its
/// probability as its outputs ([`BEST_CLASS`], [`BEST_PROB`]) instead of
/// the probabilities of every class. The class axis is the last axis of
/// the model's only output, which must be 3-dimensional
/// (`batch × steps × classes`); `ArgMax` keeps the first of equal
/// maxima, as the host-side decoding did.
///
/// # Errors
///
/// [`OcrError::Signature`] when the bytes are not an ONNX model with one
/// graph that has exactly one output.
pub(crate) fn with_best_class(model: &[u8]) -> Result<Vec<u8>, OcrError> {
    let mut out = Vec::with_capacity(model.len() + 256);
    let mut graphs = 0;
    for field in Fields::new(model) {
        let field = field?;
        if field.number == MODEL_GRAPH && field.wire == WIRE_LEN {
            graphs += 1;
            let graph = edit_graph(field.payload)?;
            put_len_field(&mut out, MODEL_GRAPH, &graph);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    if graphs != 1 {
        return Err(signature(format!("the model has {graphs} graphs, not one")));
    }
    Ok(out)
}

/// The graph with the two reductions appended and their results as its
/// only outputs.
fn edit_graph(graph: &[u8]) -> Result<Vec<u8>, OcrError> {
    let mut out = Vec::with_capacity(graph.len() + 256);
    let mut outputs = Vec::new();
    for field in Fields::new(graph) {
        let field = field?;
        if field.number == GRAPH_OUTPUT && field.wire == WIRE_LEN {
            outputs.push(value_info_name(field.payload)?);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    let [probs] = outputs.as_slice() else {
        return Err(signature(format!(
            "the recognition graph has {} outputs, not one",
            outputs.len()
        )));
    };
    // Repeated fields keep their order across the message, so these nodes
    // come after every node of the model (ONNX lists nodes in topological
    // order).
    put_len_field(
        &mut out,
        GRAPH_NODE,
        &node(
            "ArgMax",
            "blurrd_argmax",
            probs,
            BEST_CLASS,
            &axis_attrs(false),
        ),
    );
    put_len_field(
        &mut out,
        GRAPH_NODE,
        &node(
            "ReduceMax",
            "blurrd_reducemax",
            probs,
            BEST_PROB,
            &axis_attrs(true),
        ),
    );
    put_len_field(&mut out, GRAPH_OUTPUT, &value_info(BEST_CLASS, ELEM_INT64));
    put_len_field(&mut out, GRAPH_OUTPUT, &value_info(BEST_PROB, ELEM_FLOAT));
    Ok(out)
}

/// The name of a `ValueInfoProto`.
fn value_info_name(info: &[u8]) -> Result<&str, OcrError> {
    for field in Fields::new(info) {
        let field = field?;
        if field.number == VALUE_INFO_NAME && field.wire == WIRE_LEN {
            return std::str::from_utf8(field.payload)
                .map_err(|_| signature("a graph output name is not UTF-8"));
        }
    }
    Err(signature("a graph output has no name"))
}

/// The attributes of a reduction over the last axis without keeping it:
/// `axis` for `ArgMax`, `axes` (an attribute up to opset 17) for
/// `ReduceMax`.
fn axis_attrs(list: bool) -> Vec<Vec<u8>> {
    let mut axis = Vec::new();
    if list {
        put_str_field(&mut axis, ATTR_NAME, "axes");
        // -1 as a two's-complement varint.
        put_varint_field(&mut axis, ATTR_INTS, u64::MAX);
        put_varint_field(&mut axis, ATTR_TYPE, ATTR_TYPE_INTS);
    } else {
        put_str_field(&mut axis, ATTR_NAME, "axis");
        put_varint_field(&mut axis, ATTR_I, u64::MAX);
        put_varint_field(&mut axis, ATTR_TYPE, ATTR_TYPE_INT);
    }
    let mut keepdims = Vec::new();
    put_str_field(&mut keepdims, ATTR_NAME, "keepdims");
    put_varint_field(&mut keepdims, ATTR_I, 0);
    put_varint_field(&mut keepdims, ATTR_TYPE, ATTR_TYPE_INT);
    vec![axis, keepdims]
}

/// A `NodeProto` of the default domain with one input and one output.
fn node(op: &str, name: &str, input: &str, output: &str, attributes: &[Vec<u8>]) -> Vec<u8> {
    let mut node = Vec::new();
    put_str_field(&mut node, NODE_INPUT, input);
    put_str_field(&mut node, NODE_OUTPUT, output);
    put_str_field(&mut node, NODE_NAME, name);
    put_str_field(&mut node, NODE_OP_TYPE, op);
    for attribute in attributes {
        put_len_field(&mut node, NODE_ATTRIBUTE, attribute);
    }
    node
}

/// A `ValueInfoProto` of a tensor with element type `elem` and an
/// unspecified shape (the runtime infers it).
fn value_info(name: &str, elem: u64) -> Vec<u8> {
    let mut tensor = Vec::new();
    put_varint_field(&mut tensor, TENSOR_ELEM_TYPE, elem);
    let mut ty = Vec::new();
    put_len_field(&mut ty, TYPE_TENSOR, &tensor);
    let mut info = Vec::new();
    put_str_field(&mut info, VALUE_INFO_NAME, name);
    put_len_field(&mut info, VALUE_INFO_TYPE, &ty);
    info
}

fn signature(message: impl std::fmt::Display) -> OcrError {
    OcrError::Signature(format!("editing the recognition model: {message}"))
}

/// One field of a protocol buffer message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Field<'a> {
    number: u32,
    wire: u8,
    /// The payload of a length-delimited field (empty for the others).
    payload: &'a [u8],
    /// The whole field: tag, length and value.
    raw: &'a [u8],
}

/// The top-level fields of a protocol buffer message, in order.
struct Fields<'a> {
    data: &'a [u8],
    pos: usize,
    failed: bool,
}

impl<'a> Fields<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            failed: false,
        }
    }

    fn varint(&mut self) -> Result<u64, OcrError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or_else(|| signature("a varint runs past the end"))?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(signature("a varint is longer than 10 bytes"))
    }

    fn skip(&mut self, len: u64) -> Result<(), OcrError> {
        let end = usize::try_from(len)
            .ok()
            .and_then(|len| self.pos.checked_add(len))
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| signature("a field runs past the end"))?;
        self.pos = end;
        Ok(())
    }

    fn field(&mut self) -> Result<Field<'a>, OcrError> {
        let start = self.pos;
        let tag = self.varint()?;
        let number =
            u32::try_from(tag >> 3).map_err(|_| signature("a field number is out of range"))?;
        let wire = (tag & 7) as u8;
        let mut payload: &[u8] = &[];
        match wire {
            WIRE_VARINT => {
                self.varint()?;
            }
            WIRE_FIXED64 => self.skip(8)?,
            WIRE_FIXED32 => self.skip(4)?,
            WIRE_LEN => {
                let len = self.varint()?;
                let from = self.pos;
                self.skip(len)?;
                payload = &self.data[from..self.pos];
            }
            other => return Err(signature(format!("unsupported wire type {other}"))),
        }
        Ok(Field {
            number,
            wire,
            payload,
            raw: &self.data[start..self.pos],
        })
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<Field<'a>, OcrError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.pos >= self.data.len() {
            return None;
        }
        let field = self.field();
        self.failed = field.is_err();
        Some(field)
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_tag(out: &mut Vec<u8>, number: u32, wire: u8) {
    put_varint(out, (u64::from(number) << 3) | u64::from(wire));
}

fn put_varint_field(out: &mut Vec<u8>, number: u32, value: u64) {
    put_tag(out, number, WIRE_VARINT);
    put_varint(out, value);
}

fn put_len_field(out: &mut Vec<u8>, number: u32, payload: &[u8]) {
    put_tag(out, number, WIRE_LEN);
    put_varint(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

fn put_str_field(out: &mut Vec<u8>, number: u32, value: &str) {
    put_len_field(out, number, value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::{
        BEST_CLASS, BEST_PROB, Fields, GRAPH_NODE, GRAPH_OUTPUT, MODEL_GRAPH, NODE_OP_TYPE,
        put_len_field, put_str_field, put_varint_field, value_info, value_info_name,
        with_best_class,
    };

    /// A model with an IR version, one graph (a softmax node, a name and
    /// one output) and an opset import after the graph.
    fn model(outputs: &[&str]) -> Vec<u8> {
        let mut softmax = Vec::new();
        put_str_field(&mut softmax, 1, "logits");
        put_str_field(&mut softmax, 2, "probs");
        put_str_field(&mut softmax, 4, "Softmax");
        let mut graph = Vec::new();
        put_len_field(&mut graph, GRAPH_NODE, &softmax);
        put_str_field(&mut graph, 2, "rec");
        for name in outputs {
            put_len_field(&mut graph, GRAPH_OUTPUT, &value_info(name, 1));
        }
        let mut opset = Vec::new();
        put_varint_field(&mut opset, 2, 14);
        let mut model = Vec::new();
        put_varint_field(&mut model, 1, 10);
        put_len_field(&mut model, MODEL_GRAPH, &graph);
        put_len_field(&mut model, 8, &opset);
        model
    }

    fn graph_of(model: &[u8]) -> Vec<u8> {
        Fields::new(model)
            .map(Result::unwrap)
            .find(|f| f.number == MODEL_GRAPH)
            .unwrap()
            .payload
            .to_vec()
    }

    #[test]
    fn appends_the_reductions_as_the_only_outputs() {
        let original = model(&["probs"]);
        let edited = with_best_class(&original).unwrap();
        // The fields around the graph are copied unchanged, in order.
        let numbers: Vec<u32> = Fields::new(&edited).map(|f| f.unwrap().number).collect();
        assert_eq!(numbers, [1, MODEL_GRAPH, 8]);
        assert!(edited.starts_with(&original[..2]));
        assert!(edited.ends_with(&original[original.len() - 4..]));
        let graph = graph_of(&edited);
        let fields: Vec<_> = Fields::new(&graph).map(Result::unwrap).collect();
        let ops: Vec<String> = fields
            .iter()
            .filter(|f| f.number == GRAPH_NODE)
            .map(|f| {
                let op = Fields::new(f.payload)
                    .map(Result::unwrap)
                    .find(|n| n.number == NODE_OP_TYPE)
                    .unwrap();
                String::from_utf8(op.payload.to_vec()).unwrap()
            })
            .collect();
        assert_eq!(ops, ["Softmax", "ArgMax", "ReduceMax"]);
        let outputs: Vec<&str> = fields
            .iter()
            .filter(|f| f.number == GRAPH_OUTPUT)
            .map(|f| value_info_name(f.payload).unwrap())
            .collect();
        assert_eq!(outputs, [BEST_CLASS, BEST_PROB]);
        // The graph's name survives.
        assert!(fields.iter().any(|f| f.number == 2 && f.payload == b"rec"));
    }

    #[test]
    fn rejects_models_it_cannot_edit() {
        assert!(with_best_class(&model(&["a", "b"])).is_err());
        assert!(with_best_class(&model(&[])).is_err());
        // No graph at all.
        let mut no_graph = Vec::new();
        put_varint_field(&mut no_graph, 1, 10);
        assert!(with_best_class(&no_graph).is_err());
        // Truncated input.
        let full = model(&["probs"]);
        assert!(with_best_class(&full[..full.len() - 3]).is_err());
        // A group (wire type 3) is not supported.
        assert!(with_best_class(&[0x0b]).is_err());
    }
}
